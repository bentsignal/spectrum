use super::*;

impl LiveRevisionStore {
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
            let working = (|| {
                #[cfg(target_os = "linux")]
                let working_store = RevisionStore::open_publication_managed(&working_path)?;
                #[cfg(not(target_os = "linux"))]
                let working_store = RevisionStore::open(&working_path)?;
                working_store.checkpoint()?;
                let result = (
                    working_store.project_info()?,
                    working_store.generation()?,
                    working_store.state_id()?,
                );
                drop(working_store);
                Ok::<_, RevisionError>(result)
            })();
            let keep_working = match working {
                Ok((working_info, working_generation, working_state_id)) => {
                    if working_info.project_id != canonical.info.project_id {
                        return Err(RevisionError::Corrupt(
                            "live cache belongs to a different project".into(),
                        ));
                    }
                    let exact = working_generation == canonical.generation
                        && working_state_id == canonical.state_id;
                    #[cfg(target_os = "linux")]
                    let recoverable_newer = working_generation > canonical.generation
                        && working_recovery_marker_matches(
                            &PrivateDirectory::open(&project_directory)?,
                            working_generation,
                            working_state_id,
                        );
                    #[cfg(not(target_os = "linux"))]
                    let recoverable_newer = working_generation > canonical.generation;
                    exact || recoverable_newer
                }
                Err(_) => false,
            };
            if !keep_working {
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

        #[cfg(target_os = "linux")]
        let store = RevisionStore::open_publication_managed(&working_path)?;
        #[cfg(not(target_os = "linux"))]
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
            publish_capabilities: PublishCapabilities::default(),
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
            recover_full_copy(
                &PrivateDirectory::open(cache_directory)?,
                &live.canonical_path,
            )?;
        }
        if live.store.generation()? > canonical.generation {
            live.publish()?;
        }
        Ok(live)
    }

    #[cfg(target_os = "linux")]
    pub(super) fn preserved_publish_failure(
        &self,
        error: &RevisionError,
    ) -> RevisionResult<String> {
        self.store.make_checkpoint_durable()?;
        let generation = self.store.generation()?;
        let state_id = self.store.state_id()?;
        write_working_recovery_marker(
            &PrivateDirectory::open(
                self.lock_path
                    .parent()
                    .expect("publish lock always has a cache directory"),
            )?,
            generation,
            state_id,
        )?;
        Ok(error.to_string())
    }

    #[cfg(not(target_os = "linux"))]
    pub(super) fn preserved_publish_failure(
        &self,
        error: &RevisionError,
    ) -> RevisionResult<String> {
        Ok(error.to_string())
    }
}
