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
        #[cfg(target_os = "linux")]
        let private_directory = PrivateDirectory::open(&project_directory)?;
        #[cfg(target_os = "linux")]
        if working_recovery_poison_present(&private_directory)? {
            return Err(RevisionError::Invalid(
                "live cache contains an unacknowledged publication failure".into(),
            ));
        }
        if working_path.exists() {
            #[cfg(target_os = "linux")]
            let working_recovery_marker = read_working_recovery_marker(&private_directory)?;
            #[cfg(target_os = "linux")]
            let publication_marker = read_publication_marker(&private_directory)?;
            #[cfg(target_os = "linux")]
            let mut remove_stale_publication_marker = false;
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
                    let recoverable_newer = {
                        let recovery_matches_working =
                            working_recovery_marker.as_deref().is_none_or(|marker| {
                                working_recovery_marker_bytes_match(
                                    marker,
                                    working_generation,
                                    working_state_id,
                                )
                            });
                        if !recovery_matches_working {
                            return Err(RevisionError::Invalid(
                                "live cache has a working recovery marker that does not match its \
                                 working state"
                                    .into(),
                            ));
                        }
                        let publication_matches_working =
                            publication_marker.as_deref().is_some_and(|marker| {
                                publication_marker_bytes_match(
                                    marker,
                                    working_generation,
                                    working_state_id,
                                )
                            });
                        let publication_matches_canonical =
                            publication_marker.as_deref().is_some_and(|marker| {
                                publication_marker_bytes_match(
                                    marker,
                                    canonical.generation,
                                    canonical.state_id,
                                )
                            });
                        if publication_marker.is_some()
                            && !publication_matches_working
                            && !publication_matches_canonical
                        {
                            return Err(RevisionError::Invalid(
                                "live cache has a publication marker that matches neither its \
                                 canonical nor working state"
                                    .into(),
                            ));
                        }
                        let has_recovery_marker = working_recovery_marker.is_some();
                        let recoverable = working_generation > canonical.generation
                            && (has_recovery_marker || publication_matches_working);
                        if !exact && !recoverable && has_recovery_marker {
                            return Err(RevisionError::Invalid(
                                "live cache has an unresolved working recovery marker".into(),
                            ));
                        }
                        remove_stale_publication_marker = !exact
                            && !recoverable
                            && publication_matches_working
                            && !publication_matches_canonical;
                        recoverable
                    };
                    #[cfg(not(target_os = "linux"))]
                    let recoverable_newer = working_generation > canonical.generation;
                    exact || recoverable_newer
                }
                Err(_error) => {
                    #[cfg(target_os = "linux")]
                    if working_recovery_marker.is_some()
                        || publication_marker.as_deref().is_some_and(|marker| {
                            !publication_marker_bytes_match(
                                marker,
                                canonical.generation,
                                canonical.state_id,
                            )
                        })
                    {
                        return Err(RevisionError::Invalid(format!(
                            "live cache cannot be validated against its publication marker: \
                             {_error}"
                        )));
                    }
                    false
                }
            };
            if !keep_working {
                if sidecar_path(&working_path, "-shm").exists() {
                    return Err(RevisionError::Invalid(
                        "project changed elsewhere while its live cache is in use".into(),
                    ));
                }
                #[cfg(target_os = "linux")]
                if remove_stale_publication_marker {
                    remove_private_file(&private_directory, PUBLISH_CURRENT_FILE)?;
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
            publication_poisoned: Cell::new(false),
            initial_publish_pending: Cell::new(false),
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
        let installed = write_working_recovery_marker(
            &PrivateDirectory::open(
                self.lock_path
                    .parent()
                    .expect("publish lock always has a cache directory"),
            )?,
            generation,
            state_id,
        )?;
        Ok(match installed {
            WorkingRecoveryInstall::Clean => error.to_string(),
            WorkingRecoveryInstall::PoisonCleanupPending(cleanup_error) => format!(
                "{error}; recovery marker is durable but poison cleanup is pending: {cleanup_error}"
            ),
        })
    }

    #[cfg(not(target_os = "linux"))]
    pub(super) fn preserved_publish_failure(
        &self,
        error: &RevisionError,
    ) -> RevisionResult<String> {
        Ok(error.to_string())
    }

    #[cfg(target_os = "linux")]
    pub(super) fn publish_recovered_working_locked(&self) -> RevisionResult<()> {
        let directory = PrivateDirectory::open(
            self.lock_path
                .parent()
                .expect("publish lock always has a cache directory"),
        )?;
        let poisoned = working_recovery_poison_present(&directory)?;
        let generation = self.store.generation()?;
        let state_id = self.store.state_id()?;
        if let Some(marker) = read_working_recovery_marker(&directory)? {
            if !working_recovery_marker_bytes_match(&marker, generation, state_id) {
                return Err(RevisionError::Invalid(
                    "working recovery marker is malformed or does not match the live cache".into(),
                ));
            }
            if poisoned {
                if self.pending_publish_error.borrow().is_none() {
                    return Err(RevisionError::Invalid(
                        "live cache contains an unacknowledged publication failure".into(),
                    ));
                }
                remove_private_file(&directory, PUBLISH_WORKING_POISON_FILE)?;
            }
            return self.publish_recovered_current();
        }
        if poisoned {
            return Err(RevisionError::Invalid(
                "live cache contains an unacknowledged publication failure".into(),
            ));
        }
        if self.initial_publish_pending.get() {
            let destination_is_absent = match fs::metadata(&self.canonical_path) {
                Err(error) if error.kind() == io::ErrorKind::NotFound => true,
                Ok(_) => false,
                Err(error) => return Err(error.into()),
            };
            if self.published_generation.get() == 0
                && self.published_state_id.get().is_none()
                && destination_is_absent
            {
                return Ok(());
            }
            return Err(RevisionError::Invalid(
                "initial live publication no longer has an empty canonical destination".into(),
            ));
        }
        if !self.canonical_path.is_file() {
            return Err(RevisionError::Invalid(
                "published project file disappeared while its live cache remained open".into(),
            ));
        }
        let published_here = self.published_generation.get() == generation
            && self.published_state_id.get() == state_id;
        let published_predecessor_generation = self.published_generation.get();
        let published_predecessor_state_id = self.published_state_id.get();
        match read_publication_marker(&directory)? {
            Some(marker) if publication_marker_bytes_match(&marker, generation, state_id) => {
                if published_here {
                    Ok(())
                } else {
                    self.publish_recovered_current()
                }
            }
            Some(marker)
                if publication_marker_bytes_match(
                    &marker,
                    published_predecessor_generation,
                    published_predecessor_state_id,
                ) =>
            {
                self.publish_recovered_current()
            }
            Some(_) => Err(RevisionError::Invalid(
                "publication marker is malformed or matches neither the live cache nor its \
                 published predecessor"
                    .into(),
            )),
            None if published_here => Ok(()),
            None => Err(RevisionError::Invalid(
                "live cache advanced beyond its last durable publication marker".into(),
            )),
        }
    }

    #[cfg(target_os = "linux")]
    fn publish_recovered_current(&self) -> RevisionResult<()> {
        match self.publish_current_locked() {
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
}
