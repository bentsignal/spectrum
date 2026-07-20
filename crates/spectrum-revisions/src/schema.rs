use rusqlite::{Connection, params};

use crate::{RevisionError, RevisionResult, TrackId};

pub(crate) const CONTAINER_FORMAT: u32 = 3;
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

pub(crate) fn migrate(connection: &mut Connection) -> RevisionResult<()> {
    let version = container_format(connection)?;
    if version > CONTAINER_FORMAT {
        return Err(RevisionError::Invalid(format!(
            "project container format {version} is newer than supported format {CONTAINER_FORMAT}"
        )));
    }
    if version == 1 {
        migrate_v1_to_v2(connection)?;
    }
    if container_format(connection)? == 2 {
        migrate_v2_to_v3(connection)?;
    }
    connection.execute_batch(COLLABORATION_SCHEMA)?;
    Ok(())
}

fn migrate_v1_to_v2(connection: &mut Connection) -> RevisionResult<()> {
    let default_track = TrackId::new();
    let root_revision: Vec<u8> = connection.query_row(
        "SELECT value FROM spectrum_meta WHERE key = 'root_revision'",
        [],
        |row| row.get(0),
    )?;
    let created_at: i64 = String::from_utf8(connection.query_row(
        "SELECT value FROM spectrum_meta WHERE key = 'created_at_ms'",
        [],
        |row| row.get(0),
    )?)
    .map_err(|_| RevisionError::Corrupt("creation time is not UTF-8".into()))?
    .parse()
    .map_err(|_| RevisionError::Corrupt("creation time is invalid".into()))?;
    let application_id = String::from_utf8(connection.query_row(
        "SELECT value FROM spectrum_meta WHERE key = 'application_id'",
        [],
        |row| row.get(0),
    )?)
    .map_err(|_| RevisionError::Corrupt("application id is not UTF-8".into()))?;
    let transaction = connection.transaction()?;
    transaction.execute_batch(
        "CREATE TABLE tracks (
             id BLOB PRIMARY KEY CHECK(length(id) = 16),
             kind TEXT NOT NULL,
             label TEXT NOT NULL,
             root_revision_id BLOB NOT NULL UNIQUE CHECK(length(root_revision_id) = 16),
             created_at_ms INTEGER NOT NULL
         ) WITHOUT ROWID;
         ALTER TABLE revisions ADD COLUMN track_id BLOB;
         CREATE INDEX revisions_by_track ON revisions(track_id, created_at_ms, id);
         CREATE TABLE session_cursors (
             session_id BLOB NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
             track_id BLOB NOT NULL REFERENCES tracks(id),
             cursor_revision_id BLOB NOT NULL CHECK(length(cursor_revision_id) = 16),
             updated_at_ms INTEGER NOT NULL,
             PRIMARY KEY(session_id, track_id)
         ) WITHOUT ROWID;
         ALTER TABLE collaborations ADD COLUMN track_id BLOB;",
    )?;
    transaction.execute(
        "INSERT INTO tracks(id, kind, label, root_revision_id, created_at_ms)
         VALUES (?1, ?2, 'Project', ?3, ?4)",
        params![
            default_track.as_bytes().as_slice(),
            format!("{application_id}.legacy-root"),
            root_revision,
            created_at
        ],
    )?;
    transaction.execute(
        "UPDATE revisions SET track_id = ?1 WHERE track_id IS NULL",
        [default_track.as_bytes().as_slice()],
    )?;
    transaction.execute(
        "INSERT INTO session_cursors(session_id, track_id, cursor_revision_id, updated_at_ms)
         SELECT id, ?1, cursor_revision_id, updated_at_ms FROM sessions",
        [default_track.as_bytes().as_slice()],
    )?;
    transaction.execute(
        "UPDATE collaborations SET track_id = ?1 WHERE track_id IS NULL",
        [default_track.as_bytes().as_slice()],
    )?;
    transaction.execute(
        "INSERT INTO spectrum_meta(key, value) VALUES ('default_track_id', ?1)",
        [default_track.as_bytes().as_slice()],
    )?;
    crate::metadata::bump_generation(&transaction)?;
    transaction.pragma_update(None, "user_version", 2)?;
    transaction.commit()?;
    Ok(())
}

fn migrate_v2_to_v3(connection: &mut Connection) -> RevisionResult<()> {
    let transaction = connection.transaction()?;
    transaction.execute_batch(
        "ALTER TABLE revisions ADD COLUMN change_set_id BLOB;
         UPDATE revisions SET change_set_id = id WHERE change_set_id IS NULL;
         CREATE INDEX revisions_by_change_set
             ON revisions(change_set_id, created_at_ms, id);",
    )?;
    crate::metadata::bump_generation(&transaction)?;
    transaction.pragma_update(None, "user_version", CONTAINER_FORMAT)?;
    transaction.commit()?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_one_projects_gain_a_default_track_and_per_track_cursors() {
        let mut connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "PRAGMA foreign_keys = ON;
                 PRAGMA user_version = 1;
                 CREATE TABLE spectrum_meta(key TEXT PRIMARY KEY, value BLOB NOT NULL) WITHOUT ROWID;
                 CREATE TABLE sessions(
                     id BLOB PRIMARY KEY, actor_id TEXT NOT NULL, actor_name TEXT NOT NULL,
                     actor_kind TEXT NOT NULL, cursor_revision_id BLOB NOT NULL,
                     updated_at_ms INTEGER NOT NULL
                 ) WITHOUT ROWID;
                 CREATE TABLE revisions(
                     id BLOB PRIMARY KEY, parent_id BLOB REFERENCES revisions(id),
                     actor_id TEXT NOT NULL, actor_name TEXT NOT NULL, actor_kind TEXT NOT NULL,
                     session_id BLOB NOT NULL, created_at_ms INTEGER NOT NULL,
                     application_version TEXT NOT NULL, label TEXT, command_count INTEGER NOT NULL
                 ) WITHOUT ROWID;
                 CREATE TABLE collaborations(
                     agent_session_id BLOB PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,
                     source_session_id BLOB NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                     base_revision_id BLOB NOT NULL REFERENCES revisions(id),
                     followed_revision_id BLOB NOT NULL REFERENCES revisions(id),
                     mode TEXT NOT NULL, status TEXT NOT NULL,
                     created_at_ms INTEGER NOT NULL, updated_at_ms INTEGER NOT NULL
                 ) WITHOUT ROWID;",
            )
            .unwrap();
        let root = [1_u8; 16];
        let session = [2_u8; 16];
        for (key, value) in [
            ("project_id", [3_u8; 16].to_vec()),
            ("application_id", b"spectrum.lumen".to_vec()),
            ("created_at_ms", b"123".to_vec()),
            ("root_revision", root.to_vec()),
            ("storage_generation", b"4".to_vec()),
        ] {
            connection
                .execute(
                    "INSERT INTO spectrum_meta(key, value) VALUES (?1, ?2)",
                    params![key, value],
                )
                .unwrap();
        }
        connection
            .execute(
                "INSERT INTO revisions VALUES (?1, NULL, 'person:1', 'Person', 'human', ?2, 123, '1', 'Root', 0)",
                params![root, session],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO sessions VALUES (?1, 'person:1', 'Person', 'human', ?2, 123)",
                params![session, root],
            )
            .unwrap();

        migrate(&mut connection).unwrap();

        assert_eq!(container_format(&connection).unwrap(), 3);
        let default_track: Vec<u8> = connection
            .query_row(
                "SELECT value FROM spectrum_meta WHERE key = 'default_track_id'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(default_track.len(), 16);
        let revision_track: Vec<u8> = connection
            .query_row(
                "SELECT track_id FROM revisions WHERE id = ?1",
                [root],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(revision_track, default_track);
        let cursor: Vec<u8> = connection
            .query_row(
                "SELECT cursor_revision_id FROM session_cursors
                 WHERE session_id = ?1 AND track_id = ?2",
                params![session, default_track],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(cursor, root);
    }
}
