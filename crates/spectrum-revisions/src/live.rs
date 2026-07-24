use std::{
    cell::{Cell, RefCell},
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
    time::Instant,
};

use fs2::FileExt;
#[cfg(target_os = "linux")]
use std::io;

use crate::{
    NewProject, ProjectInfo, RevisionError, RevisionResult, RevisionStore, SessionId,
    metadata::StorageStateId, storage_io::sidecar_path,
};

mod capabilities;
#[cfg(target_os = "linux")]
mod fault;
#[cfg(target_os = "linux")]
mod linux_io;
mod metrics;
#[cfg(all(test, target_os = "linux"))]
mod mutation_tests;
mod no_replace;
mod recovery;
#[cfg(all(test, target_os = "linux"))]
mod recovery_tests;
#[cfg(all(test, target_os = "linux"))]
mod tail_tests;
#[cfg(test)]
mod tests;
use capabilities::PublishCapabilities;
#[cfg(target_os = "linux")]
use fault::*;
#[cfg(target_os = "linux")]
use linux_io::{
    ExchangeIntent, PrivateDirectory, PublishBase, WorkingRecoveryInstall, catch_up_after_bulk,
    exchange_proof_matches, open_unnamed, prepare_reusable_slot, publication_marker_bytes_match,
    publish_unnamed, publish_unnamed_no_replace, read_publication_marker,
    read_working_recovery_marker, recover_exchange, recover_full_copy, remove_private_file,
    seed_incremental_mirror, update_candidate_slot, validate_publish_base,
    working_recovery_marker_bytes_match, working_recovery_poison_present, write_full_copy_intent,
    write_publication_marker, write_working_recovery_marker,
};
use metrics::elapsed_us;
pub use metrics::{PublishStats, PublishStrategy, PublishTimings};
use no_replace::rename_no_replace;

const STORE_FILE: &str = "live.sqlite";
const LOCK_FILE: &str = "publish.lock";
#[cfg(target_os = "linux")]
const PUBLISH_MIRROR_FILE: &str = "published-mirror.sqlite";
#[cfg(target_os = "linux")]
const PUBLISH_MIRROR_READY_FILE: &str = "published-mirror.ready";
#[cfg(target_os = "linux")]
const PUBLISH_EXCHANGE_INTENT_FILE: &str = "published-exchange.intent";
#[cfg(target_os = "linux")]
const PUBLISH_CURRENT_FILE: &str = "published-current.ready";
#[cfg(target_os = "linux")]
const PUBLISH_FULL_COPY_INTENT_FILE: &str = "published-copy.intent";
#[cfg(target_os = "linux")]
const PUBLISH_WORKING_RECOVERY_FILE: &str = "working-recovery.ready";
#[cfg(target_os = "linux")]
const PUBLISH_WORKING_POISON_FILE: &str = "working-recovery.poison";
#[cfg(target_os = "linux")]
const LEGACY_PUBLISH_BACKUP_FILE: &str = "published-backup.sqlite";
#[cfg(target_os = "linux")]
const WRITE_BLOCK_BYTES: usize = 4 * 1024;
#[cfg(target_os = "linux")]
const BULK_CHANGE_CATCH_UP_BYTES: u64 = 1024 * 1024;

/// A live SQLite working store whose user-facing project is always a single checkpointed file.
///
/// WAL and shared-memory files stay in the app-owned cache directory. After every mutation the
/// checkpointed main database is atomically published to `canonical_path`.
pub struct LiveRevisionStore {
    store: RevisionStore,
    canonical_path: PathBuf,
    working_path: PathBuf,
    lock_path: PathBuf,
    published_generation: Cell<u64>,
    published_state_id: Cell<Option<StorageStateId>>,
    temps_cleaned: Cell<bool>,
    pending_publish_error: RefCell<Option<String>>,
    publication_poisoned: Cell<bool>,
    initial_publish_pending: Cell<bool>,
    last_publish_stats: Cell<PublishStats>,
    publish_capabilities: PublishCapabilities,
}

impl LiveRevisionStore {
    pub fn create(
        canonical_path: &Path,
        cache_root: &Path,
        project: NewProject,
    ) -> RevisionResult<(Self, ProjectInfo)> {
        let canonical_path = absolute_path(canonical_path)?;
        if canonical_path.exists() {
            return Err(RevisionError::Invalid(format!(
                "refusing to replace existing project {}",
                canonical_path.display()
            )));
        }
        create_private_dir_all(cache_root)?;
        let staging = cache_root.join(format!("staging-{}", SessionId::new()));
        fs::create_dir(&staging)?;
        make_private(&staging)?;
        let staging_path = staging.join(STORE_FILE);
        let created = RevisionStore::create(&staging_path, project);
        let (store, info) = match created {
            Ok(created) => created,
            Err(error) => {
                let _ = fs::remove_dir_all(&staging);
                return Err(error);
            }
        };
        store.checkpoint()?;
        drop(store);

        let project_directory = cache_root.join(info.project_id.to_string());
        if project_directory.exists() {
            let _ = fs::remove_dir_all(&staging);
            return Err(RevisionError::Invalid(format!(
                "live cache already exists for project {}",
                info.project_id
            )));
        }
        fs::rename(&staging, &project_directory)?;
        let working_path = project_directory.join(STORE_FILE);
        #[cfg(target_os = "linux")]
        let store = RevisionStore::open_publication_managed(&working_path)?;
        #[cfg(not(target_os = "linux"))]
        let store = RevisionStore::open(&working_path)?;
        let live = Self {
            store,
            canonical_path,
            working_path,
            lock_path: project_directory.join(LOCK_FILE),
            published_generation: Cell::new(0),
            published_state_id: Cell::new(None),
            temps_cleaned: Cell::new(false),
            pending_publish_error: RefCell::new(None),
            publication_poisoned: Cell::new(false),
            initial_publish_pending: Cell::new(true),
            last_publish_stats: Cell::new(PublishStats::default()),
            publish_capabilities: PublishCapabilities::default(),
        };
        live.publish()?;
        Ok((live, info))
    }

    pub fn store(&self) -> &RevisionStore {
        &self.store
    }

    pub fn mutate<T>(
        &mut self,
        mutation: impl FnOnce(&mut RevisionStore) -> RevisionResult<T>,
    ) -> RevisionResult<T> {
        self.require_publishable()?;
        #[cfg(not(target_os = "linux"))]
        if self.pending_publish_error.borrow().is_some() {
            self.publish()?;
        }
        #[cfg(target_os = "linux")]
        let _cache_lock = lock_private(&self.lock_path)?;
        #[cfg(target_os = "linux")]
        self.publish_recovered_working_locked()?;
        let core_started = Instant::now();
        let result = mutation(&mut self.store)?;
        let core_write_us = elapsed_us(core_started.elapsed());
        #[cfg(target_os = "linux")]
        let published = self.publish_current_locked();
        #[cfg(not(target_os = "linux"))]
        let published = self.publish_current();
        match published {
            Ok(()) => {
                let mut stats = self.last_publish_stats.get();
                stats.timings.core_write_us = core_write_us;
                self.last_publish_stats.set(stats);
                self.pending_publish_error.replace(None)
            }
            Err(error) => match self.preserved_publish_failure(&error) {
                Ok(message) => self.pending_publish_error.replace(Some(message)),
                Err(recovery_error) => {
                    let message =
                        format!("{error}; could not preserve failed publication: {recovery_error}");
                    self.pending_publish_error.replace(Some(message.clone()));
                    self.publication_poisoned.set(true);
                    return Err(RevisionError::Invalid(message));
                }
            },
        };
        Ok(result)
    }

    pub fn publish(&self) -> RevisionResult<()> {
        self.require_publishable()?;
        #[cfg(target_os = "linux")]
        let _cache_lock = lock_private(&self.lock_path)?;
        #[cfg(target_os = "linux")]
        self.publish_recovered_working_locked()?;
        self.store.checkpoint()?;
        #[cfg(target_os = "linux")]
        let published = self.publish_current_locked();
        #[cfg(not(target_os = "linux"))]
        let published = self.publish_current();
        match published {
            Ok(()) => {
                self.pending_publish_error.replace(None);
                Ok(())
            }
            Err(error) => {
                match self.preserved_publish_failure(&error) {
                    Ok(message) => self.pending_publish_error.replace(Some(message)),
                    Err(recovery_error) => {
                        let message = format!(
                            "{error}; could not preserve failed publication: {recovery_error}"
                        );
                        self.pending_publish_error.replace(Some(message.clone()));
                        self.publication_poisoned.set(true);
                        return Err(RevisionError::Invalid(message));
                    }
                };
                Err(error)
            }
        }
    }

    #[cfg(not(target_os = "linux"))]
    fn publish_current(&self) -> RevisionResult<()> {
        let _lock = lock(&self.lock_path)?;
        self.publish_current_locked()
    }

    fn publish_current_locked(&self) -> RevisionResult<()> {
        #[cfg(target_os = "linux")]
        let _canonical_lock = lock_directory(
            self.canonical_path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .unwrap_or_else(|| Path::new(".")),
        )?;
        if !self.temps_cleaned.get() {
            cleanup_publish_temps(&self.canonical_path)?;
            self.temps_cleaned.set(true);
        }
        let working_generation = self.store.generation()?;
        if self.canonical_path.is_file() && self.published_generation.get() >= working_generation {
            self.last_publish_stats.set(PublishStats::default());
            #[cfg(target_os = "linux")]
            remove_private_file(
                &PrivateDirectory::open(
                    self.lock_path
                        .parent()
                        .expect("publish lock always has a cache directory"),
                )?,
                PUBLISH_WORKING_RECOVERY_FILE,
            )?;
            self.initial_publish_pending.set(false);
            return Ok(());
        }
        let stats = publish_checkpoint(
            &self.working_path,
            &self.canonical_path,
            self.lock_path
                .parent()
                .expect("publish lock always has a cache directory"),
            &self.publish_capabilities,
            self.published_generation.get(),
            self.published_state_id.get(),
            self.initial_publish_pending.get(),
        )?;
        self.last_publish_stats.set(stats);
        self.published_generation.set(working_generation);
        self.published_state_id
            .set(RevisionStore::inspect(&self.working_path)?.state_id);
        #[cfg(target_os = "linux")]
        remove_private_file(
            &PrivateDirectory::open(
                self.lock_path
                    .parent()
                    .expect("publish lock always has a cache directory"),
            )?,
            PUBLISH_WORKING_RECOVERY_FILE,
        )?;
        self.initial_publish_pending.set(false);
        Ok(())
    }

    pub fn canonical_path(&self) -> &Path {
        &self.canonical_path
    }

    pub fn working_path(&self) -> &Path {
        &self.working_path
    }

    pub fn pending_publish_error(&self) -> Option<String> {
        self.pending_publish_error.borrow().clone()
    }

    pub fn last_publish_stats(&self) -> PublishStats {
        self.last_publish_stats.get()
    }

    fn require_publishable(&self) -> RevisionResult<()> {
        if self.publication_poisoned.get() {
            return Err(RevisionError::Invalid(
                self.pending_publish_error
                    .borrow()
                    .clone()
                    .unwrap_or_else(|| "live publication state is poisoned".into()),
            ));
        }
        Ok(())
    }
}

fn absolute_path(path: &Path) -> RevisionResult<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_owned())
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

fn create_private_dir_all(path: &Path) -> RevisionResult<()> {
    fs::create_dir_all(path)?;
    make_private(path)
}

#[cfg(unix)]
fn make_private(path: &Path) -> RevisionResult<()> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

#[cfg(not(unix))]
fn make_private(_path: &Path) -> RevisionResult<()> {
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn recover_legacy_sidecars(path: &Path) -> RevisionResult<()> {
    if sidecar_path(path, "-wal").exists() || sidecar_path(path, "-shm").exists() {
        let store = RevisionStore::open(path)?;
        store.checkpoint()?;
        drop(store);
        if sidecar_path(path, "-shm").exists() {
            return Err(RevisionError::Invalid(
                "project is already open by a legacy Spectrum process".into(),
            ));
        }
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn prepare_canonical_copy(
    canonical_path: &Path,
    cache_root: &Path,
) -> RevisionResult<crate::store::StoreInspection> {
    if sidecar_path(canonical_path, "-shm").exists() {
        return Err(RevisionError::Invalid(
            "project is already open by a legacy Spectrum process".into(),
        ));
    }
    create_private_dir_all(cache_root)?;
    let staging = cache_root.join(format!(".canonical-prepare-{}.sqlite", SessionId::new()));
    let prepared = (|| {
        RevisionStore::snapshot_for_migration(canonical_path, &staging)?;
        atomic_publish(&staging, canonical_path, false)?;
        sync_parent(canonical_path)?;
        RevisionStore::inspect(canonical_path)
    })();
    remove_sidecars(&staging).ok();
    let _ = fs::remove_file(&staging);
    prepared
}

fn remove_sidecars(path: &Path) -> RevisionResult<()> {
    for suffix in ["-wal", "-shm"] {
        let sidecar = sidecar_path(path, suffix);
        match fs::remove_file(sidecar) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn lock(path: &Path) -> RevisionResult<File> {
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)?;
    file.lock_exclusive()?;
    Ok(file)
}

#[cfg(target_os = "linux")]
fn lock_private(path: &Path) -> RevisionResult<File> {
    let parent = path
        .parent()
        .ok_or_else(|| RevisionError::Invalid("publish lock has no private directory".into()))?;
    // Opening the lock descriptor must not repair the mutation namespace: another
    // publisher may hold this same lock with its candidate sealed inside the gate.
    let directory = PrivateDirectory::open_unrecovered(parent)?;
    let lock = match directory.create_file(LOCK_FILE) {
        Ok(lock) => lock,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            directory.open_file(LOCK_FILE, true)?
        }
        Err(error) => return Err(error.into()),
    };
    let identity = validated_identity(&lock, true)?;
    maybe_private_lock_hardlink(path)?;
    use std::os::unix::fs::PermissionsExt as _;
    if lock.metadata()?.permissions().mode() & 0o777 != 0o600 {
        return Err(RevisionError::Invalid(
            "publish lock must have private owner-only permissions".into(),
        ));
    }
    directory.validate(LOCK_FILE, identity, true)?;
    lock.lock_exclusive()?;
    directory.validate(LOCK_FILE, identity, true)?;
    directory.recover_mutation()?;
    Ok(lock)
}

#[cfg(target_os = "linux")]
fn lock_directory(path: &Path) -> RevisionResult<File> {
    use std::os::unix::fs::OpenOptionsExt as _;

    let directory = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW)
        .open(path)?;
    directory.lock_exclusive()?;
    Ok(directory)
}

fn replace_with_copy(source: &Path, destination: &Path) -> RevisionResult<()> {
    let temporary = temporary_path(destination);
    if let Err(error) = copy_or_clone(source, &temporary).and_then(|_| {
        File::open(&temporary)?.sync_all()?;
        fs::rename(&temporary, destination)
    }) {
        let _ = fs::remove_file(&temporary);
        return Err(error.into());
    }
    Ok(())
}

fn atomic_publish(
    source: &Path,
    destination: &Path,
    initial_no_replace: bool,
) -> RevisionResult<()> {
    if let Some(parent) = destination
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }
    #[cfg(target_os = "linux")]
    {
        let parent = destination
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        if let Ok(mut candidate) = open_unnamed(parent) {
            let mut source_file = open_nofollow(source, false)?;
            io::copy(&mut source_file, &mut candidate)?;
            let permissions = destination
                .metadata()
                .or_else(|_| source.metadata())
                .map(|metadata| metadata.permissions())?;
            candidate.set_permissions(permissions)?;
            candidate.sync_all()?;
            if initial_no_replace {
                publish_unnamed_no_replace(candidate, destination)?;
            } else {
                publish_unnamed(candidate, destination)?;
            }
            return Ok(());
        }
    }

    atomic_publish_named(source, destination, initial_no_replace)
}

fn atomic_publish_named(
    source: &Path,
    destination: &Path,
    initial_no_replace: bool,
) -> RevisionResult<()> {
    let temporary = temporary_path(destination);
    let permissions = destination
        .metadata()
        .ok()
        .map(|metadata| metadata.permissions());
    if let Err(error) = copy_or_clone(source, &temporary).and_then(|_| {
        if let Some(permissions) = permissions {
            fs::set_permissions(&temporary, permissions)?;
        }
        File::open(&temporary)?.sync_all()?;
        if initial_no_replace {
            rename_no_replace(&temporary, destination)
        } else {
            fs::rename(&temporary, destination)
        }
    }) {
        let _ = fs::remove_file(&temporary);
        return Err(error.into());
    }
    Ok(())
}

fn publish_checkpoint(
    source: &Path,
    destination: &Path,
    _cache_directory: &Path,
    _publish_capabilities: &PublishCapabilities,
    _expected_generation: u64,
    _expected_state_id: Option<StorageStateId>,
    initial_publish: bool,
) -> RevisionResult<PublishStats> {
    #[cfg(target_os = "linux")]
    {
        let preparation_started = Instant::now();
        let source_inspection = RevisionStore::inspect(source)?;
        let directory = PrivateDirectory::open(_cache_directory)?;
        recover_exchange(&directory, destination, _cache_directory)?;
        recover_full_copy(&directory, destination)?;
        let base = validate_publish_base(
            destination,
            _cache_directory,
            _expected_generation,
            _expected_state_id,
            source_inspection.generation,
            source_inspection.state_id,
        )?;
        if base.generation == source_inspection.generation
            && base.state_id == source_inspection.state_id
        {
            write_publication_marker(
                &directory,
                source_inspection.generation,
                source_inspection.state_id,
            )?;
            return Ok(PublishStats::default());
        }
        let preparation_us = elapsed_us(preparation_started.elapsed());
        let (incremental, mut reflink_unavailable) = incremental_publish(
            source,
            destination,
            _cache_directory,
            _publish_capabilities,
            &base,
            source_inspection.generation,
            source_inspection.state_id,
        )?;
        if let Some(mut stats) = incremental {
            stats.timings.preparation_us = preparation_us;
            return Ok(stats);
        }
        reflink_unavailable |= open_unnamed(_cache_directory).is_err();
        write_full_copy_intent(
            &directory,
            base.generation,
            base.state_id,
            source_inspection.generation,
            source_inspection.state_id,
        )?;
        if initial_publish {
            maybe_initial_publish_race(destination)?;
        }
        atomic_publish(source, destination, initial_publish)?;
        sync_parent(destination)?;
        maybe_publish_fault(PublishFault::FullCopyPublished)?;
        write_publication_marker(
            &directory,
            source_inspection.generation,
            source_inspection.state_id,
        )?;
        remove_private_file(&directory, PUBLISH_FULL_COPY_INTENT_FILE)?;
        let written_bytes = source.metadata()?.len();
        seed_incremental_mirror(
            destination,
            _cache_directory,
            source_inspection.generation,
            source_inspection.state_id,
        );
        Ok(PublishStats {
            incremental: false,
            reflink_unavailable,
            strategy: PublishStrategy::FullCopy,
            scanned_bytes: written_bytes,
            changed_bytes: written_bytes,
            written_bytes,
            timings: PublishTimings {
                preparation_us,
                ..PublishTimings::default()
            },
        })
    }

    #[cfg(not(target_os = "linux"))]
    atomic_publish(source, destination, initial_publish)?;
    #[cfg(not(target_os = "linux"))]
    let written_bytes = source.metadata()?.len();
    #[cfg(not(target_os = "linux"))]
    Ok(PublishStats {
        incremental: false,
        reflink_unavailable: false,
        strategy: PublishStrategy::FullCopy,
        scanned_bytes: written_bytes,
        changed_bytes: written_bytes,
        written_bytes,
        timings: PublishTimings::default(),
    })
}

#[cfg(target_os = "linux")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FileIdentity {
    device: u64,
    inode: u64,
}

#[cfg(target_os = "linux")]
fn incremental_publish(
    source: &Path,
    destination: &Path,
    cache_directory: &Path,
    publish_capabilities: &PublishCapabilities,
    base: &PublishBase,
    source_generation: u64,
    source_state_id: Option<StorageStateId>,
) -> RevisionResult<(Option<PublishStats>, bool)> {
    let candidate_started = Instant::now();
    let (Some(_), Some(canonical_identity), Some(canonical_permissions)) = (
        base.canonical.as_ref(),
        base.identity,
        base.permissions.clone(),
    ) else {
        return Ok((None, false));
    };
    let directory = PrivateDirectory::open(cache_directory)?;
    use std::os::unix::fs::MetadataExt as _;
    if base
        .canonical
        .as_ref()
        .expect("incremental publication requires a canonical descriptor")
        .metadata()?
        .nlink()
        != 1
    {
        return Ok((None, false));
    }
    remove_private_file(&directory, LEGACY_PUBLISH_BACKUP_FILE)?;
    if destination.metadata()?.dev() != directory.device()? {
        return Ok((None, false));
    }
    let exchange_probe_started = Instant::now();
    let exchange_supported =
        publish_capabilities.exchange_supported(|| directory.exchange_supported())?;
    if !exchange_supported {
        return Ok((None, false));
    }
    let exchange_probe_us = elapsed_us(exchange_probe_started.elapsed());
    let mirror_file = match directory.open_file(PUBLISH_MIRROR_FILE, true) {
        Ok(file) => file,
        Err(_) => return Ok((None, false)),
    };
    let mirror_identity = validated_identity(&mirror_file, true)?;
    if mirror_identity == canonical_identity {
        return Ok((None, false));
    }
    remove_private_file(&directory, PUBLISH_MIRROR_READY_FILE)?;
    let intent = ExchangeIntent {
        canonical_identity,
        candidate_identity: mirror_identity,
        generation: base.generation,
        state_id: base.state_id,
        target_generation: source_generation,
        target_state_id: source_state_id,
    };
    let Some(mut stats) = update_candidate_slot(
        &directory,
        source,
        &mirror_file,
        mirror_identity,
        canonical_permissions,
        intent,
    )?
    else {
        return Ok((None, false));
    };
    stats.timings.exchange_probe_us = exchange_probe_us;
    stats.timings.candidate_us = elapsed_us(candidate_started.elapsed());
    maybe_publish_fault(PublishFault::CandidateSynced)?;

    let intent_started = Instant::now();
    maybe_publish_fault(PublishFault::IntentCreated)?;
    directory.validate(PUBLISH_MIRROR_FILE, mirror_identity, true)?;
    validate_named_identity(destination, canonical_identity, false)?;
    stats.timings.intent_us = elapsed_us(intent_started.elapsed());
    maybe_publish_fault(PublishFault::PreExchangeValidated)?;
    let exchange_started = Instant::now();
    let old_slot_private = match directory.exchange(
        PUBLISH_MIRROR_FILE,
        mirror_identity,
        destination,
        canonical_identity,
    )? {
        linux_io::ExchangeOutcome::CanonicalLinked => {
            return Ok((None, false));
        }
        linux_io::ExchangeOutcome::Exchanged { old_slot_private } => old_slot_private,
    };
    stats.timings.exchange_us = elapsed_us(exchange_started.elapsed());
    maybe_publish_fault(PublishFault::Exchanged)?;
    if !old_slot_private {
        let cleanup_started = Instant::now();
        remove_private_file(&directory, PUBLISH_MIRROR_FILE)?;
        maybe_publish_fault(PublishFault::LinkedSlotUnlinked)?;
        remove_private_file(&directory, PUBLISH_MIRROR_READY_FILE)?;
        stats.timings.intent_cleanup_us = elapsed_us(cleanup_started.elapsed());
        stats.incremental = true;
        stats.strategy = PublishStrategy::PageDiffExchange;
        return Ok((Some(stats), false));
    }

    let old_canonical = base
        .canonical
        .as_ref()
        .expect("incremental publication requires a canonical descriptor");
    let slot_started = Instant::now();
    prepare_reusable_slot(
        &directory,
        PUBLISH_MIRROR_FILE,
        old_canonical,
        canonical_identity,
    )?;
    stats.timings.slot_prepare_us = elapsed_us(slot_started.elapsed());
    maybe_publish_fault(PublishFault::SlotWritable)?;
    maybe_publish_fault(PublishFault::MarkerCreated)?;
    let cleanup_started = Instant::now();
    stats.timings.intent_cleanup_us = elapsed_us(cleanup_started.elapsed());
    maybe_publish_fault(PublishFault::IntentRemoved)?;

    let catch_up_started = Instant::now();
    stats = catch_up_after_bulk(
        &directory,
        source,
        stats,
        source_generation,
        source_state_id,
    )?;
    stats.timings.catch_up_us = elapsed_us(catch_up_started.elapsed());

    stats.incremental = true;
    stats.strategy = PublishStrategy::PageDiffExchange;
    Ok((Some(stats), false))
}

#[cfg(target_os = "linux")]
fn open_nofollow(path: &Path, write: bool) -> std::io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;

    let mut options = OpenOptions::new();
    options
        .read(true)
        .write(write)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW);
    options.open(path)
}

#[cfg(target_os = "linux")]
fn openat_nofollow(directory: &File, name: &std::ffi::CStr, write: bool) -> std::io::Result<File> {
    use std::os::fd::{AsRawFd as _, FromRawFd as _};

    let mut flags = libc::O_CLOEXEC | libc::O_NOFOLLOW;
    flags |= if write { libc::O_RDWR } else { libc::O_RDONLY };
    let descriptor = unsafe { libc::openat(directory.as_raw_fd(), name.as_ptr(), flags) };
    if descriptor < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(unsafe { File::from_raw_fd(descriptor) })
    }
}

#[cfg(target_os = "linux")]
fn file_identity(metadata: &fs::Metadata) -> FileIdentity {
    use std::os::unix::fs::MetadataExt;

    FileIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
    }
}

#[cfg(target_os = "linux")]
fn validated_identity(file: &File, private: bool) -> RevisionResult<FileIdentity> {
    use std::os::unix::fs::MetadataExt;

    let metadata = file.metadata()?;
    if !metadata.file_type().is_file() || (private && metadata.nlink() != 1) {
        return Err(RevisionError::Invalid(
            "publication file is not a private regular inode".into(),
        ));
    }
    Ok(file_identity(&metadata))
}

#[cfg(target_os = "linux")]
fn validate_named_identity(
    path: &Path,
    expected: FileIdentity,
    private: bool,
) -> RevisionResult<()> {
    let actual = open_nofollow(path, false)?;
    if validated_identity(&actual, private)? != expected {
        return Err(RevisionError::Invalid(
            "publication path changed before its atomic rename".into(),
        ));
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn sync_parent(path: &Path) -> RevisionResult<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    sync_directory(parent)
}

#[cfg(target_os = "linux")]
fn sync_directory(path: &Path) -> RevisionResult<()> {
    File::open(path)?.sync_all()?;
    Ok(())
}

fn temporary_path(destination: &Path) -> PathBuf {
    let file_name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("project");
    destination.with_file_name(format!(
        ".{file_name}.spectrum-publish-{}.tmp",
        SessionId::new()
    ))
}

fn cleanup_publish_temps(destination: &Path) -> RevisionResult<()> {
    let Some(parent) = destination.parent() else {
        return Ok(());
    };
    let file_name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("project");
    let prefix = format!(".{file_name}.spectrum-publish-");
    for entry in fs::read_dir(parent)? {
        let path = entry?.path();
        let matches = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with(&prefix) && name.ends_with(".tmp"));
        if matches {
            fs::remove_file(path)?;
        }
    }
    Ok(())
}

fn copy_or_clone(source: &Path, destination: &Path) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    if clone_file(source, destination).is_ok() {
        return Ok(());
    }
    fs::copy(source, destination).map(|_| ())
}

#[cfg(target_os = "macos")]
fn clone_file(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};

    unsafe extern "C" {
        fn clonefile(
            source: *const std::ffi::c_char,
            destination: *const std::ffi::c_char,
            flags: u32,
        ) -> std::ffi::c_int;
    }

    let source = CString::new(source.as_os_str().as_bytes())?;
    let destination = CString::new(destination.as_os_str().as_bytes())?;
    if unsafe { clonefile(source.as_ptr(), destination.as_ptr(), 0) } == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}
