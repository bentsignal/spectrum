use std::{error::Error, fmt};

use crate::{RevisionId, SessionId};

pub type RevisionResult<T> = Result<T, RevisionError>;

#[derive(Debug)]
pub enum RevisionError {
    Io(std::io::Error),
    Database(rusqlite::Error),
    Serialization(serde_json::Error),
    NotARevisionStore,
    AlreadyInitialized,
    MissingRevision(RevisionId),
    MissingSession(SessionId),
    CursorMoved {
        session: SessionId,
        expected: RevisionId,
        actual: RevisionId,
    },
    IncompatibleRevision(RevisionId),
    Corrupt(String),
    Invalid(String),
}

impl fmt::Display for RevisionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "revision storage error: {error}"),
            Self::Database(error) => write!(formatter, "revision database error: {error}"),
            Self::Serialization(error) => write!(formatter, "revision payload error: {error}"),
            Self::NotARevisionStore => write!(formatter, "file is not a Spectrum revision store"),
            Self::AlreadyInitialized => write!(formatter, "revision store is already initialized"),
            Self::MissingRevision(id) => write!(formatter, "revision {id} does not exist"),
            Self::MissingSession(id) => write!(formatter, "session {id} does not exist"),
            Self::CursorMoved {
                session,
                expected,
                actual,
            } => write!(
                formatter,
                "session {session} moved from expected revision {expected} to {actual}"
            ),
            Self::IncompatibleRevision(id) => {
                write!(formatter, "revision {id} has no compatible replay path")
            }
            Self::Corrupt(message) => write!(formatter, "revision store is corrupt: {message}"),
            Self::Invalid(message) => write!(formatter, "invalid revision data: {message}"),
        }
    }
}

impl Error for RevisionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Database(error) => Some(error),
            Self::Serialization(error) => Some(error),
            _ => None,
        }
    }
}

impl From<std::io::Error> for RevisionError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<rusqlite::Error> for RevisionError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Database(error)
    }
}

impl From<serde_json::Error> for RevisionError {
    fn from(error: serde_json::Error) -> Self {
        Self::Serialization(error)
    }
}
