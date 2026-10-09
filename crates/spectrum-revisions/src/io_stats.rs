//! Counts of storage work, so interaction tests can bound how much disk
//! work each edit, open, or check does. Durability flushes are cheap on some
//! systems and very slow on others (macOS flushes the whole drive), so
//! tests count them instead of trusting one machine's timings.
use std::sync::atomic::{AtomicU64, Ordering};

static LIVE_OPENS: AtomicU64 = AtomicU64::new(0);
static STORE_OPENS: AtomicU64 = AtomicU64::new(0);
static SYNCS: AtomicU64 = AtomicU64::new(0);
static PUBLICATIONS: AtomicU64 = AtomicU64::new(0);

/// Storage work done by this process so far.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct IoStats {
    /// Documents opened through their live cache.
    pub live_opens: u64,
    /// SQLite revision stores opened, read-only ones included.
    pub store_opens: u64,
    /// Files flushed to disk.
    pub syncs: u64,
    /// Document files written back after a change.
    pub publications: u64,
}

impl IoStats {
    /// The work done since `earlier`.
    pub fn since(self, earlier: IoStats) -> IoStats {
        IoStats {
            live_opens: self.live_opens - earlier.live_opens,
            store_opens: self.store_opens - earlier.store_opens,
            syncs: self.syncs - earlier.syncs,
            publications: self.publications - earlier.publications,
        }
    }
}

pub fn io_stats() -> IoStats {
    IoStats {
        live_opens: LIVE_OPENS.load(Ordering::Relaxed),
        store_opens: STORE_OPENS.load(Ordering::Relaxed),
        syncs: SYNCS.load(Ordering::Relaxed),
        publications: PUBLICATIONS.load(Ordering::Relaxed),
    }
}

pub(crate) fn live_opened() {
    LIVE_OPENS.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn store_opened() {
    STORE_OPENS.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn published() {
    PUBLICATIONS.fetch_add(1, Ordering::Relaxed);
}

/// `File::sync_all`, counted.
pub(crate) trait SyncCounted {
    fn sync_all_counted(&self) -> std::io::Result<()>;
}

impl SyncCounted for std::fs::File {
    fn sync_all_counted(&self) -> std::io::Result<()> {
        SYNCS.fetch_add(1, Ordering::Relaxed);
        self.sync_all()
    }
}
