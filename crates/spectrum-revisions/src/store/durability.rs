use super::{RevisionStore, WriteDurability};
use crate::{
    RevisionResult,
    storage_io::{sidecar_path, sync_if_present},
};
use std::fs;
#[cfg(target_os = "linux")]
use std::path::Path;

impl RevisionStore {
    #[cfg(target_os = "linux")]
    pub(crate) fn open_publication_managed(path: &Path) -> RevisionResult<Self> {
        Self::open_with_durability(path, WriteDurability::PublicationManaged)
    }

    pub fn checkpoint(&self) -> RevisionResult<()> {
        self.finish_write()
    }

    pub(crate) fn finish_write(&self) -> RevisionResult<()> {
        // An empty log means every write was already checkpointed and flushed,
        // so opening or reading a document flushes nothing; flushes are slow
        // on macOS.
        let written = fs::metadata(sidecar_path(&self.path, "-wal")).is_ok_and(|wal| wal.len() > 0);
        self.connection
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")?;
        if written && self.syncs_checkpoint_files() {
            self.sync_checkpoint_files()?;
        }
        Ok(())
    }

    #[cfg(target_os = "linux")]
    pub(crate) fn make_checkpoint_durable(&self) -> RevisionResult<()> {
        self.connection
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")?;
        self.sync_checkpoint_files()
    }

    fn syncs_checkpoint_files(&self) -> bool {
        match self.durability {
            WriteDurability::Standalone => true,
            #[cfg(target_os = "linux")]
            WriteDurability::PublicationManaged => false,
        }
    }

    fn sync_checkpoint_files(&self) -> RevisionResult<()> {
        sync_if_present(&self.path)?;
        sync_if_present(&sidecar_path(&self.path, "-wal"))?;
        Ok(())
    }
}
