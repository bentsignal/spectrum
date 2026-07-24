use rusqlite::{Connection, OptionalExtension as _};

use crate::{
    Actor, ActorKind, ChangeSetId, ProjectId, ProjectInfo, Revision, RevisionError, RevisionId,
    RevisionResult, Session, SessionId, TrackId, schema, store_tracks::track_id,
};

pub(crate) fn session_in(
    connection: &Connection,
    id: SessionId,
) -> RevisionResult<Option<Session>> {
    connection
        .query_row(
            "SELECT actor_id, actor_name, actor_kind, cursor_revision_id, updated_at_ms
             FROM sessions WHERE id = ?1",
            [id.as_bytes().as_slice()],
            |row| {
                let kind: String = row.get(2)?;
                Ok(Session {
                    id,
                    actor: Actor {
                        id: row.get(0)?,
                        display_name: row.get(1)?,
                        kind: actor_kind(&kind)?,
                    },
                    cursor: revision_id(row.get(3)?)?,
                    updated_at_ms: row.get(4)?,
                })
            },
        )
        .optional()
        .map_err(Into::into)
}

pub(super) fn revision_in(
    connection: &Connection,
    id: RevisionId,
) -> RevisionResult<Option<Revision>> {
    connection
        .query_row(
            "SELECT id, track_id, change_set_id, parent_id, actor_id, actor_name, actor_kind, session_id,
                    created_at_ms, application_version, label, command_count
             FROM revisions WHERE id = ?1",
            [id.as_bytes().as_slice()],
            revision_from_row,
        )
        .optional()
        .map_err(Into::into)
}

pub(super) fn revision_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Revision> {
    let kind: String = row.get(6)?;
    Ok(Revision {
        id: revision_id(row.get(0)?)?,
        track_id: track_id(row.get(1)?)?,
        change_set_id: change_set_id(row.get(2)?)?,
        parent_id: row
            .get::<_, Option<Vec<u8>>>(3)?
            .map(revision_id)
            .transpose()?,
        actor: Actor {
            id: row.get(4)?,
            display_name: row.get(5)?,
            kind: actor_kind(&kind)?,
        },
        session_id: session_id(row.get(7)?)?,
        created_at_ms: row.get(8)?,
        application_version: row.get(9)?,
        label: row.get(10)?,
        command_count: row.get(11)?,
    })
}

pub(super) fn require_revision_in(connection: &Connection, id: RevisionId) -> RevisionResult<()> {
    let exists: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM revisions WHERE id = ?1)",
        [id.as_bytes().as_slice()],
        |row| row.get(0),
    )?;
    if exists {
        Ok(())
    } else {
        Err(RevisionError::MissingRevision(id))
    }
}

pub(super) fn project_info_in(connection: &Connection) -> RevisionResult<ProjectInfo> {
    let required = |key: &str| -> RevisionResult<Vec<u8>> {
        connection
            .query_row(
                "SELECT value FROM spectrum_meta WHERE key = ?1",
                [key],
                |row| row.get(0),
            )
            .optional()?
            .ok_or_else(|| RevisionError::Corrupt(format!("missing metadata key {key}")))
    };
    let project_id = ProjectId::from_bytes(array::<16>(&required("project_id")?, "project id")?);
    let application_id = String::from_utf8(required("application_id")?)
        .map_err(|_| RevisionError::Corrupt("application id is not UTF-8".into()))?;
    let created_at_ms = String::from_utf8(required("created_at_ms")?)
        .map_err(|_| RevisionError::Corrupt("creation time is not UTF-8".into()))?
        .parse()
        .map_err(|_| RevisionError::Corrupt("creation time is invalid".into()))?;
    let root_revision =
        RevisionId::from_bytes(array::<16>(&required("root_revision")?, "root revision")?);
    let default_track_id = TrackId::from_bytes(array::<16>(
        &required("default_track_id")?,
        "default track id",
    )?);
    Ok(ProjectInfo {
        project_id,
        application_id,
        created_at_ms,
        container_format: schema::container_format(connection)?,
        default_track_id,
        root_revision,
    })
}

pub(crate) fn actor_kind(value: &str) -> rusqlite::Result<ActorKind> {
    ActorKind::parse(value).ok_or_else(|| conversion_error("invalid actor kind"))
}

pub(crate) fn revision_id(bytes: Vec<u8>) -> rusqlite::Result<RevisionId> {
    Ok(RevisionId::from_bytes(sql_array::<16>(
        bytes,
        "revision id",
    )?))
}

pub(super) fn session_id(bytes: Vec<u8>) -> rusqlite::Result<SessionId> {
    Ok(SessionId::from_bytes(sql_array::<16>(bytes, "session id")?))
}

fn change_set_id(bytes: Vec<u8>) -> rusqlite::Result<ChangeSetId> {
    Ok(ChangeSetId::from_bytes(sql_array::<16>(
        bytes,
        "change set id",
    )?))
}

fn sql_array<const N: usize>(bytes: Vec<u8>, label: &str) -> rusqlite::Result<[u8; N]> {
    bytes
        .try_into()
        .map_err(|_| conversion_error(&format!("invalid {label}")))
}

fn array<const N: usize>(bytes: &[u8], label: &str) -> RevisionResult<[u8; N]> {
    bytes
        .try_into()
        .map_err(|_| RevisionError::Corrupt(format!("invalid {label}")))
}

fn conversion_error(message: &str) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        0,
        rusqlite::types::Type::Blob,
        Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            message.to_owned(),
        )),
    )
}
