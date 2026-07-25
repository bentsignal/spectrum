use std::{fmt, io};

#[derive(Debug)]
pub enum BridgeError {
    Io(io::Error),
    Json(serde_json::Error),
    Protocol(String),
    Authentication(String),
    Limit(String),
    RateLimited {
        retry_after_millis: u64,
        disconnect: bool,
    },
    StaleBinding,
    CursorConflict,
    ResyncRequired {
        oldest_seq: u64,
        newest_seq: u64,
    },
    Closed,
    Unsupported(&'static str),
}

pub type BridgeResult<T> = Result<T, BridgeError>;

impl fmt::Display for BridgeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => error.fmt(formatter),
            Self::Json(error) => error.fmt(formatter),
            Self::Protocol(message) => write!(formatter, "protocol error: {message}"),
            Self::Authentication(message) => write!(formatter, "authentication failed: {message}"),
            Self::Limit(message) => write!(formatter, "transport limit exceeded: {message}"),
            Self::RateLimited {
                retry_after_millis,
                disconnect,
            } => write!(
                formatter,
                "rate limited; retry after {retry_after_millis} ms{}",
                if *disconnect { " and reconnect" } else { "" }
            ),
            Self::StaleBinding => formatter.write_str("binding is stale"),
            Self::CursorConflict => formatter.write_str("expected cursor does not match"),
            Self::ResyncRequired {
                oldest_seq,
                newest_seq,
            } => write!(
                formatter,
                "event history gap; available sequence is {oldest_seq}..={newest_seq}"
            ),
            Self::Closed => formatter.write_str("connection closed"),
            Self::Unsupported(message) => write!(formatter, "unsupported: {message}"),
        }
    }
}

impl std::error::Error for BridgeError {}

impl From<io::Error> for BridgeError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for BridgeError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}
