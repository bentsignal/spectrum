use rusqlite::Connection;

use crate::{RevisionError, RevisionResult};

pub(crate) const CONTAINER_FORMAT: u32 = 1;
const APPLICATION_ID: i64 = 0x5350_4354;
const COLLABORATION_SCHEMA: &str = "CREATE TABLE IF NOT EXISTS collaborations (
         agent_session_id BLOB PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,
         source_session_id BLOB NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
         track_id BLOB NOT NULL REFERENCES tracks(id),
         base_revision_id BLOB NOT NULL REFERENCES revisions(id),
         followed_revision_id BLOB NOT NULL REFERENCES revisions(id),
         mode TEXT NOT NULL CHECK(mode IN ('together', 'separate')),
         status TEXT NOT NULL CHECK(status IN ('active', 'split', 'superseded')),
         created_at_ms INTEGER NOT NULL,
         updated_at_ms INTEGER NOT NULL
     ) WITHOUT ROWID;
     CREATE INDEX IF NOT EXISTS collaborations_by_source
         ON collaborations(source_session_id, status, updated_at_ms);";

pub(crate) fn configure(connection: &Connection) -> RevisionResult<()> {
    // SQLite's synchronous modes fsync the parent directory. macOS can block that operation for a
    // document opened from a protected user folder even though the document and its WAL are both
    // writable. Spectrum explicitly checkpoints and fsyncs the exact project files after writes,
    // retaining WAL's checksummed crash recovery without requiring parent-directory access.
    connection.execute_batch(
        "PRAGMA foreign_keys = ON;
         PRAGMA journal_mode = WAL;
         PRAGMA synchronous = OFF;
         PRAGMA wal_autocheckpoint = 0;
         PRAGMA busy_timeout = 5000;
         PRAGMA trusted_schema = OFF;",
    )?;
    Ok(())
}

pub(crate) fn initialize(connection: &Connection) -> RevisionResult<()> {
    connection.execute_batch(&format!(
        "PRAGMA application_id = {APPLICATION_ID};
         PRAGMA user_version = {CONTAINER_FORMAT};
         CREATE TABLE IF NOT EXISTS spectrum_meta (
             key TEXT PRIMARY KEY,
             value BLOB NOT NULL
         ) WITHOUT ROWID;
         CREATE TABLE IF NOT EXISTS sessions (
             id BLOB PRIMARY KEY CHECK(length(id) = 16),
             actor_id TEXT NOT NULL,
             actor_name TEXT NOT NULL,
             actor_kind TEXT NOT NULL,
             cursor_revision_id BLOB NOT NULL CHECK(length(cursor_revision_id) = 16),
             updated_at_ms INTEGER NOT NULL
         ) WITHOUT ROWID;
         CREATE TABLE IF NOT EXISTS tracks (
             id BLOB PRIMARY KEY CHECK(length(id) = 16),
             kind TEXT NOT NULL,
             label TEXT NOT NULL,
             root_revision_id BLOB NOT NULL UNIQUE CHECK(length(root_revision_id) = 16),
             created_at_ms INTEGER NOT NULL
         ) WITHOUT ROWID;
         CREATE TABLE IF NOT EXISTS session_cursors (
             session_id BLOB NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
             track_id BLOB NOT NULL REFERENCES tracks(id),
             cursor_revision_id BLOB NOT NULL CHECK(length(cursor_revision_id) = 16),
             updated_at_ms INTEGER NOT NULL,
             PRIMARY KEY(session_id, track_id)
         ) WITHOUT ROWID;
         CREATE TABLE IF NOT EXISTS session_child_choices (
             session_id BLOB NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
             parent_revision_id BLOB NOT NULL REFERENCES revisions(id),
             child_revision_id BLOB NOT NULL REFERENCES revisions(id),
             PRIMARY KEY(session_id, parent_revision_id)
         ) WITHOUT ROWID;
         CREATE TABLE IF NOT EXISTS revisions (
             id BLOB PRIMARY KEY CHECK(length(id) = 16),
             track_id BLOB NOT NULL REFERENCES tracks(id),
             change_set_id BLOB NOT NULL CHECK(length(change_set_id) = 16),
             parent_id BLOB REFERENCES revisions(id),
             actor_id TEXT NOT NULL,
             actor_name TEXT NOT NULL,
             actor_kind TEXT NOT NULL,
             session_id BLOB NOT NULL CHECK(length(session_id) = 16),
             created_at_ms INTEGER NOT NULL,
             application_version TEXT NOT NULL,
             label TEXT,
             command_count INTEGER NOT NULL CHECK(command_count >= 0)
         ) WITHOUT ROWID;
         CREATE INDEX IF NOT EXISTS revisions_by_parent
             ON revisions(parent_id, created_at_ms, id);
         CREATE INDEX IF NOT EXISTS revisions_by_track
             ON revisions(track_id, created_at_ms, id);
         CREATE INDEX IF NOT EXISTS revisions_by_change_set
             ON revisions(change_set_id, created_at_ms, id);
         CREATE TABLE IF NOT EXISTS operation_payloads (
             revision_id BLOB NOT NULL REFERENCES revisions(id) ON DELETE CASCADE,
             family TEXT NOT NULL,
             version INTEGER NOT NULL CHECK(version >= 0),
             capabilities_json TEXT NOT NULL,
             bytes BLOB NOT NULL,
             sha256 BLOB NOT NULL CHECK(length(sha256) = 32),
             PRIMARY KEY(revision_id, family, version, capabilities_json)
         ) WITHOUT ROWID;
         CREATE TABLE IF NOT EXISTS snapshots (
             revision_id BLOB NOT NULL REFERENCES revisions(id) ON DELETE CASCADE,
             family TEXT NOT NULL,
             version INTEGER NOT NULL CHECK(version >= 0),
             capabilities_json TEXT NOT NULL,
             bytes BLOB NOT NULL,
             sha256 BLOB NOT NULL CHECK(length(sha256) = 32),
             PRIMARY KEY(revision_id, family, version, capabilities_json)
         ) WITHOUT ROWID;
         CREATE TABLE IF NOT EXISTS assets (
             sha256 BLOB PRIMARY KEY CHECK(length(sha256) = 32),
             media_type TEXT NOT NULL,
             byte_length INTEGER NOT NULL CHECK(byte_length >= 0),
             bytes BLOB NOT NULL
         ) WITHOUT ROWID;
         CREATE TABLE IF NOT EXISTS previews (
             revision_id BLOB NOT NULL REFERENCES revisions(id) ON DELETE CASCADE,
             format TEXT NOT NULL,
             width INTEGER NOT NULL CHECK(width > 0),
             height INTEGER NOT NULL CHECK(height > 0),
             bytes BLOB NOT NULL,
             sha256 BLOB NOT NULL CHECK(length(sha256) = 32),
             PRIMARY KEY(revision_id, format, width, height)
         ) WITHOUT ROWID;"
    ))?;
    connection.execute_batch(COLLABORATION_SCHEMA)?;
    Ok(())
}

/// Spectrum reads only its current container format.
pub(crate) fn check_format(connection: &Connection) -> RevisionResult<()> {
    let version = container_format(connection)?;
    if version != CONTAINER_FORMAT {
        return Err(RevisionError::Invalid(format!(
            "this document uses container format {version}; this Spectrum reads format {CONTAINER_FORMAT}"
        )));
    }
    Ok(())
}

pub(crate) fn verify_header(connection: &Connection) -> RevisionResult<()> {
    let application_id: i64 =
        connection.query_row("PRAGMA application_id", [], |row| row.get(0))?;
    if application_id != APPLICATION_ID {
        return Err(RevisionError::NotARevisionStore);
    }
    Ok(())
}

pub(crate) fn container_format(connection: &Connection) -> RevisionResult<u32> {
    connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(Into::into)
}
