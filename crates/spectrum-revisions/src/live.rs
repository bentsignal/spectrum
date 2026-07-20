use std::{
    cell::{Cell, RefCell},
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
};

use fs2::FileExt;

use crate::{
    NewProject, ProjectInfo, RevisionError, RevisionResult, RevisionStore, SessionId,
    storage_io::sidecar_path,
};

const STORE_FILE: &str = "live.sqlite";
const LOCK_FILE: &str = "publish.lock";

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
    temps_cleaned: Cell<bool>,
    pending_publish_error: RefCell<Option<String>>,
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
            temps_cleaned: Cell::new(false),
            pending_publish_error: RefCell::new(None),
        };
        live.publish()?;
        Ok((live, info))
    }

    pub fn open(canonical_path: &Path, cache_root: &Path) -> RevisionResult<Self> {
        let canonical_path = absolute_path(canonical_path)?;
        recover_legacy_sidecars(&canonical_path)?;
        // Container migrations happen against the portable file first. Incremented storage
        // generation then invalidates any older live cache before it can diverge.
        let canonical_store = RevisionStore::open(&canonical_path)?;
        canonical_store.checkpoint()?;
        drop(canonical_store);
        let canonical = RevisionStore::inspect(&canonical_path)?;
        create_private_dir_all(cache_root)?;
        let project_directory = cache_root.join(canonical.info.project_id.to_string());
        create_private_dir_all(&project_directory)?;
        let working_path = project_directory.join(STORE_FILE);
        let lock_path = project_directory.join(LOCK_FILE);
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
            temps_cleaned: Cell::new(false),
            pending_publish_error: RefCell::new(None),
        };
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
        let result = mutation(&mut self.store)?;
        match self.publish_current() {
            Ok(()) => self.pending_publish_error.replace(None),
            Err(error) => self.pending_publish_error.replace(Some(error.to_string())),
        };
        Ok(result)
    }

    pub fn publish(&self) -> RevisionResult<()> {
        self.store.checkpoint()?;
        match self.publish_current() {
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

    fn publish_current(&self) -> RevisionResult<()> {
        let _lock = lock(&self.lock_path)?;
        if !self.temps_cleaned.get() {
            cleanup_publish_temps(&self.canonical_path)?;
            self.temps_cleaned.set(true);
        }
        let working_generation = self.store.generation()?;
        if self.canonical_path.is_file() && self.published_generation.get() >= working_generation {
            return Ok(());
        }
        atomic_publish(&self.working_path, &self.canonical_path)?;
        self.published_generation.set(working_generation);
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
