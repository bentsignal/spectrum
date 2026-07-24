use std::time::Duration;

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
    pub timings: PublishTimings,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PublishTimings {
    pub core_write_us: u64,
    pub preparation_us: u64,
    pub exchange_probe_us: u64,
    pub candidate_us: u64,
    pub intent_us: u64,
    pub exchange_us: u64,
    pub current_marker_us: u64,
    pub slot_prepare_us: u64,
    pub ready_marker_us: u64,
    pub intent_cleanup_us: u64,
    pub catch_up_us: u64,
}

pub(super) fn elapsed_us(duration: Duration) -> u64 {
    u64::try_from(duration.as_micros()).unwrap_or(u64::MAX)
}
