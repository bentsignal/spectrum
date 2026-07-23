use std::{
    cell::{Cell, RefCell},
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
};

use fs2::FileExt;
#[cfg(target_os = "linux")]
use std::io;

use crate::{
    NewProject, ProjectInfo, RevisionError, RevisionResult, RevisionStore, SessionId,
    metadata::StorageStateId, storage_io::sidecar_path,
};

#[cfg(target_os = "linux")]
mod linux_io;
#[cfg(test)]
mod tests;
#[cfg(target_os = "linux")]
use linux_io::write_ready_marker;
#[cfg(target_os = "linux")]
use linux_io::{
    ExchangeIntent, PrivateDirectory, apply_checkpoint_delta, open_unnamed, publish_unnamed,
    recover_exchange, remove_private_file, seed_incremental_mirror,
};

const STORE_FILE: &str = "live.sqlite";
const LOCK_FILE: &str = "publish.lock";
#[cfg(target_os = "linux")]
const PUBLISH_MIRROR_FILE: &str = "published-mirror.sqlite";
#[cfg(target_os = "linux")]
const PUBLISH_MIRROR_READY_FILE: &str = "published-mirror.ready";
#[cfg(target_os = "linux")]
const PUBLISH_EXCHANGE_INTENT_FILE: &str = "published-exchange.intent";
#[cfg(target_os = "linux")]
const LEGACY_PUBLISH_BACKUP_FILE: &str = "published-backup.sqlite";
#[cfg(target_os = "linux")]
const WRITE_BLOCK_BYTES: usize = 4 * 1024;
#[cfg(target_os = "linux")]
const BULK_CHANGE_CATCH_UP_BYTES: u64 = 1024 * 1024;

#[cfg(target_os = "linux")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PublishFault {
    CandidateSynced,
    IntentCreated,
    PreExchangeValidated,
    Exchanged,
    SlotWritable,
    MarkerCreated,
    IntentRemoved,
    SeedMirrorCreated,
}

#[cfg(all(test, target_os = "linux"))]
#[derive(Clone, Copy)]
enum CrashMode {
    Exit,
    Abort,
    Kill,
}

#[cfg(all(test, target_os = "linux"))]
thread_local! {
    static PUBLISH_FAULT: Cell<Option<PublishFault>> = const { Cell::new(None) };
    static PUBLISH_CRASH_MODE: Cell<Option<CrashMode>> = const { Cell::new(None) };
    static PUBLISH_HARDLINK_ALIAS: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PublishStrategy {
    #[default]
    None,
    FullCopy,
    PageDiffExchange,
}

impl PublishStrategy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::FullCopy => "full-copy",
            Self::PageDiffExchange => "page-diff-exchange",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PublishStats {
    pub incremental: bool,
    pub reflink_unavailable: bool,
    pub strategy: PublishStrategy,
    pub scanned_bytes: u64,
    /// Bytes whose contents differ from the previous checkpoint.
    pub changed_bytes: u64,
    /// File-data bytes physically submitted by publication. Reflink-only publication writes zero.
    pub written_bytes: u64,
}

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
    last_publish_stats: Cell<PublishStats>,
}

impl LiveRevisionStore {
    pub fn create(
        canonical_path: &Path,
        cache_root: &Path,
        project: NewProject,
    ) -> RevisionResult<(Self, ProjectInfo)> {
        let canonical_path = absolute_path(canonical_path)?;
        if canonical_path.exists() && canonical_path.metadata()?.len() > 0 {
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
            last_publish_stats: Cell::new(PublishStats::default()),
        };
        live.publish()?;
        Ok((live, info))
    }

    pub fn open(canonical_path: &Path, cache_root: &Path) -> RevisionResult<Self> {
        let canonical_path = absolute_path(canonical_path)?;
        let canonical = if !sidecar_path(&canonical_path, "-wal").exists()
            && !sidecar_path(&canonical_path, "-shm").exists()
        {
            RevisionStore::inspect(&canonical_path).ok()
        } else {
            None
        };
        let canonical = if let Some(canonical) = canonical {
            canonical
        } else {
            #[cfg(target_os = "linux")]
            {
                let _canonical_lock = lock_directory(
                    canonical_path
                        .parent()
                        .filter(|parent| !parent.as_os_str().is_empty())
                        .unwrap_or_else(|| Path::new(".")),
                )?;
                prepare_canonical_copy(&canonical_path, cache_root)?
            }
            #[cfg(not(target_os = "linux"))]
            {
                recover_legacy_sidecars(&canonical_path)?;
                // Container migrations happen against the portable file first. Incremented
                // storage generation then invalidates an older live cache before it can diverge.
                let canonical_store = RevisionStore::open(&canonical_path)?;
                canonical_store.checkpoint()?;
                drop(canonical_store);
                RevisionStore::inspect(&canonical_path)
            }?
        };
        create_private_dir_all(cache_root)?;
        let project_directory = cache_root.join(canonical.info.project_id.to_string());
        create_private_dir_all(&project_directory)?;
        let working_path = project_directory.join(STORE_FILE);
        let lock_path = project_directory.join(LOCK_FILE);
        #[cfg(target_os = "linux")]
        let lock = lock_private(&lock_path)?;
        #[cfg(not(target_os = "linux"))]
        let lock = lock(&lock_path)?;
        if working_path.exists() {
            let working_store = RevisionStore::open(&working_path)?;
            working_store.checkpoint()?;
            let working_info = working_store.project_info()?;
            let working_generation = working_store.generation()?;
            let working_state_id = working_store.state_id()?;
            drop(working_store);
            if working_info.project_id != canonical.info.project_id {
                return Err(RevisionError::Corrupt(
                    "live cache belongs to a different project".into(),
                ));
            }
            let canonical_is_different_peer = canonical.generation == working_generation
                && canonical.state_id.is_some()
                && working_state_id.is_some()
                && canonical.state_id != working_state_id;
            if canonical.generation > working_generation || canonical_is_different_peer {
                if sidecar_path(&working_path, "-shm").exists() {
                    return Err(RevisionError::Invalid(
                        "project changed elsewhere while its live cache is in use".into(),
                    ));
                }
                remove_sidecars(&working_path)?;
                replace_with_copy(&canonical_path, &working_path)?;
            }
        } else {
            replace_with_copy(&canonical_path, &working_path)?;
        }
        drop(lock);

        let store = RevisionStore::open(&working_path)?;
        let live = Self {
            store,
            canonical_path,
            working_path,
            lock_path,
            published_generation: Cell::new(canonical.generation),
            published_state_id: Cell::new(canonical.state_id),
            temps_cleaned: Cell::new(false),
            pending_publish_error: RefCell::new(None),
            last_publish_stats: Cell::new(PublishStats::default()),
        };
        #[cfg(target_os = "linux")]
        {
            let _cache_lock = lock_private(&live.lock_path)?;
            let _canonical_lock = lock_directory(
                live.canonical_path
                    .parent()
                    .filter(|parent| !parent.as_os_str().is_empty())
                    .unwrap_or_else(|| Path::new(".")),
            )?;
            let cache_directory = live
                .lock_path
                .parent()
                .expect("publish lock always has a cache directory");
            recover_exchange(
                &PrivateDirectory::open(cache_directory)?,
                &live.canonical_path,
                cache_directory,
            )?;
        }
        if live.store.generation()? > canonical.generation {
            live.publish()?;
        }
        Ok(live)
    }

    pub fn store(&self) -> &RevisionStore {
        &self.store
    }

    pub fn mutate<T>(
        &mut self,
        mutation: impl FnOnce(&mut RevisionStore) -> RevisionResult<T>,
    ) -> RevisionResult<T> {
        #[cfg(target_os = "linux")]
        let _cache_lock = lock_private(&self.lock_path)?;
        let result = mutation(&mut self.store)?;
        #[cfg(target_os = "linux")]
        let published = self.publish_current_locked();
        #[cfg(not(target_os = "linux"))]
        let published = self.publish_current();
        match published {
            Ok(()) => self.pending_publish_error.replace(None),
            Err(error) => self.pending_publish_error.replace(Some(error.to_string())),
        };
        Ok(result)
    }

    pub fn publish(&self) -> RevisionResult<()> {
        #[cfg(target_os = "linux")]
        let _cache_lock = lock_private(&self.lock_path)?;
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
                self.pending_publish_error.replace(Some(error.to_string()));
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
            return Ok(());
        }
        let stats = publish_checkpoint(
            &self.working_path,
            &self.canonical_path,
            self.lock_path
                .parent()
                .expect("publish lock always has a cache directory"),
            self.published_generation.get(),
            self.published_state_id.get(),
        )?;
        self.last_publish_stats.set(stats);
        self.published_generation.set(working_generation);
        self.published_state_id
            .set(RevisionStore::inspect(&self.working_path)?.state_id);
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
        atomic_publish(&staging, canonical_path)?;
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
    let directory = PrivateDirectory::open(parent)?;
    let lock = match directory.create_file(LOCK_FILE) {
        Ok(lock) => lock,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            directory.open_file(LOCK_FILE, true)?
        }
        Err(error) => return Err(error.into()),
    };
    let identity = validated_identity(&lock, true)?;
    lock.set_permissions(std::os::unix::fs::PermissionsExt::from_mode(0o600))?;
    directory.validate(LOCK_FILE, identity, true)?;
    lock.lock_exclusive()?;
    directory.validate(LOCK_FILE, identity, true)?;
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

fn atomic_publish(source: &Path, destination: &Path) -> RevisionResult<()> {
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
            publish_unnamed(candidate, destination)?;
            return Ok(());
        }
    }

    atomic_publish_named(source, destination)
}

fn atomic_publish_named(source: &Path, destination: &Path) -> RevisionResult<()> {
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
        fs::rename(&temporary, destination)
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
    _expected_generation: u64,
    _expected_state_id: Option<StorageStateId>,
) -> RevisionResult<PublishStats> {
    #[cfg(target_os = "linux")]
    {
        let source_inspection = RevisionStore::inspect(source)?;
        let base = validate_publish_base(
            destination,
            _cache_directory,
            _expected_generation,
            _expected_state_id,
            source_inspection.generation,
            source_inspection.state_id,
        )?;
        let (incremental, mut reflink_unavailable) = incremental_publish(
            source,
            destination,
            _cache_directory,
            &base,
            source_inspection.generation,
            source_inspection.state_id,
        )?;
        if let Some(stats) = incremental {
            return Ok(stats);
        }
        reflink_unavailable |= open_unnamed(_cache_directory).is_err();
        atomic_publish(source, destination)?;
        sync_parent(destination)?;
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
        })
    }

    #[cfg(not(target_os = "linux"))]
    atomic_publish(source, destination)?;
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
    })
}

#[cfg(target_os = "linux")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FileIdentity {
    device: u64,
    inode: u64,
}

#[cfg(target_os = "linux")]
struct PublishBase {
    canonical: Option<File>,
    identity: Option<FileIdentity>,
    permissions: Option<fs::Permissions>,
    generation: u64,
    state_id: Option<StorageStateId>,
}

#[cfg(target_os = "linux")]
fn validate_publish_base(
    destination: &Path,
    _cache_directory: &Path,
    expected_generation: u64,
    expected_state_id: Option<StorageStateId>,
    source_generation: u64,
    source_state_id: Option<StorageStateId>,
) -> RevisionResult<PublishBase> {
    let canonical = match open_nofollow(destination, false) {
        Ok(file) => file,
        Err(error)
            if error.kind() == std::io::ErrorKind::NotFound
                && expected_generation == 0
                && expected_state_id.is_none() =>
        {
            return Ok(PublishBase {
                canonical: None,
                identity: None,
                permissions: None,
                generation: 0,
                state_id: None,
            });
        }
        Err(error) => return Err(error.into()),
    };
    let metadata = canonical.metadata()?;
    if !metadata.file_type().is_file() {
        return Err(RevisionError::Invalid(
            "canonical project is not a regular file".into(),
        ));
    }
    let identity = file_identity(&metadata);
    let inspection = RevisionStore::inspect(destination)?;
    let revalidated = open_nofollow(destination, false)?;
    if validated_identity(&revalidated, false)? != identity {
        return Err(RevisionError::Invalid(
            "canonical project changed while it was inspected".into(),
        ));
    }
    let expected_matches =
        inspection.generation == expected_generation && inspection.state_id == expected_state_id;
    let already_published =
        inspection.generation == source_generation && inspection.state_id == source_state_id;
    if !expected_matches && !already_published {
        return Err(RevisionError::Invalid(format!(
            "project publication conflict: expected generation {expected_generation}, found {}",
            inspection.generation
        )));
    }
    Ok(PublishBase {
        canonical: Some(canonical),
        identity: Some(identity),
        permissions: Some(metadata.permissions()),
        generation: inspection.generation,
        state_id: inspection.state_id,
    })
}

#[cfg(target_os = "linux")]
fn incremental_publish(
    source: &Path,
    destination: &Path,
    cache_directory: &Path,
    base: &PublishBase,
    source_generation: u64,
    source_state_id: Option<StorageStateId>,
) -> RevisionResult<(Option<PublishStats>, bool)> {
    let (Some(_), Some(canonical_identity), Some(canonical_permissions)) = (
        base.canonical.as_ref(),
        base.identity,
        base.permissions.clone(),
    ) else {
        return Ok((None, false));
    };
    let directory = PrivateDirectory::open(cache_directory)?;
    recover_exchange(&directory, destination, cache_directory)?;
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
    if !directory.exchange_supported()? {
        return Ok((None, false));
    }
    let mirror_file = match directory.open_file(PUBLISH_MIRROR_FILE, true) {
        Ok(file) => file,
        Err(_) => return Ok((None, false)),
    };
    let mirror_identity = validated_identity(&mirror_file, true)?;
    if mirror_identity == canonical_identity {
        return Ok((None, false));
    }
    remove_private_file(&directory, PUBLISH_MIRROR_READY_FILE)?;
    let mut stats = apply_checkpoint_delta(source, &mirror_file)?;
    mirror_file.set_permissions(canonical_permissions)?;
    mirror_file.sync_all()?;
    directory.validate(PUBLISH_MIRROR_FILE, mirror_identity, true)?;
    maybe_publish_fault(PublishFault::CandidateSynced)?;

    let intent = ExchangeIntent {
        canonical_identity,
        candidate_identity: mirror_identity,
        generation: base.generation,
        state_id: base.state_id,
        target_generation: source_generation,
        target_state_id: source_state_id,
    };
    directory.write_marker(PUBLISH_EXCHANGE_INTENT_FILE, &intent.encode())?;
    maybe_publish_fault(PublishFault::IntentCreated)?;
    directory.validate(PUBLISH_MIRROR_FILE, mirror_identity, true)?;
    validate_named_identity(destination, canonical_identity, false)?;
    maybe_publish_fault(PublishFault::PreExchangeValidated)?;
    let old_slot_private = match directory.exchange(
        PUBLISH_MIRROR_FILE,
        mirror_identity,
        destination,
        canonical_identity,
    )? {
        linux_io::ExchangeOutcome::CanonicalLinked => {
            remove_private_file(&directory, PUBLISH_EXCHANGE_INTENT_FILE)?;
            return Ok((None, false));
        }
        linux_io::ExchangeOutcome::Exchanged { old_slot_private } => old_slot_private,
    };
    maybe_publish_fault(PublishFault::Exchanged)?;

    if !old_slot_private {
        remove_private_file(&directory, PUBLISH_MIRROR_FILE)?;
        remove_private_file(&directory, PUBLISH_MIRROR_READY_FILE)?;
        remove_private_file(&directory, PUBLISH_EXCHANGE_INTENT_FILE)?;
        directory.sync()?;
        stats.incremental = true;
        stats.strategy = PublishStrategy::PageDiffExchange;
        return Ok((Some(stats), false));
    }

    directory.validate(PUBLISH_MIRROR_FILE, canonical_identity, true)?;
    let old_canonical = base
        .canonical
        .as_ref()
        .expect("incremental publication requires a canonical descriptor");
    old_canonical.set_permissions(std::os::unix::fs::PermissionsExt::from_mode(0o600))?;
    old_canonical.sync_all()?;
    maybe_publish_fault(PublishFault::SlotWritable)?;
    write_ready_marker(&directory, base.generation, base.state_id)?;
    maybe_publish_fault(PublishFault::MarkerCreated)?;
    remove_private_file(&directory, PUBLISH_EXCHANGE_INTENT_FILE)?;
    directory.sync()?;
    maybe_publish_fault(PublishFault::IntentRemoved)?;

    if stats.changed_bytes >= BULK_CHANGE_CATCH_UP_BYTES
        && stats.changed_bytes.saturating_mul(4) >= stats.scanned_bytes
    {
        let caught_up_slot = directory.open_file(PUBLISH_MIRROR_FILE, true)?;
        let caught_up_identity = validated_identity(&caught_up_slot, true)?;
        let catch_up = apply_checkpoint_delta(source, &caught_up_slot)?;
        caught_up_slot.sync_all()?;
        directory.validate(PUBLISH_MIRROR_FILE, caught_up_identity, true)?;
        write_ready_marker(&directory, source_generation, source_state_id)?;
        stats.scanned_bytes = stats.scanned_bytes.saturating_add(catch_up.scanned_bytes);
        stats.changed_bytes = stats.changed_bytes.saturating_add(catch_up.changed_bytes);
        stats.written_bytes = stats.written_bytes.saturating_add(catch_up.written_bytes);
    }

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

#[cfg(all(test, target_os = "linux"))]
fn maybe_publish_fault(point: PublishFault) -> RevisionResult<()> {
    if PUBLISH_FAULT.get() == Some(point) {
        if let Some(mode) = PUBLISH_CRASH_MODE.get() {
            match mode {
                CrashMode::Exit => unsafe { libc::_exit(86) },
                CrashMode::Abort => std::process::abort(),
                CrashMode::Kill => unsafe {
                    libc::raise(libc::SIGKILL);
                    libc::_exit(87);
                },
            }
        }
        return Err(RevisionError::Invalid(format!(
            "injected publication fault at {point:?}"
        )));
    }
    Ok(())
}

#[cfg(all(not(test), target_os = "linux"))]
fn maybe_publish_fault(_point: PublishFault) -> RevisionResult<()> {
    Ok(())
}

#[cfg(all(test, target_os = "linux"))]
fn maybe_seed_fault() -> RevisionResult<()> {
    maybe_publish_fault(PublishFault::SeedMirrorCreated)
}

#[cfg(all(not(test), target_os = "linux"))]
fn maybe_seed_fault() -> RevisionResult<()> {
    maybe_publish_fault(PublishFault::SeedMirrorCreated)
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
