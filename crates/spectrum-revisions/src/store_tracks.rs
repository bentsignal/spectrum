use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};

use crate::{
    Actor, AppendRevision, ChangeSetId, NewTrack, Revision, RevisionError, RevisionId,
    RevisionResult, Session, SessionId, Track, TrackId,
    metadata::bump_generation,
    storage_io::now_ms,
    store::{
        RevisionStore, actor_kind, insert_assets, insert_payloads, revision_id, session_in,
        validate_assets, validate_payload,
    },
};

impl RevisionStore {
    pub fn session_on_track(
        &self,
        id: SessionId,
        track: TrackId,
    ) -> RevisionResult<Option<Session>> {
        session_on_track_in(&self.connection, id, track)
    }

    pub fn move_session(
        &mut self,
        session_id: SessionId,
        expected_current: RevisionId,
        target: RevisionId,
    ) -> RevisionResult<Session> {
        let target_revision = self
            .revision(target)?
            .ok_or(RevisionError::MissingRevision(target))?;
        let track = target_revision.track_id;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut session = session_on_track_in(&transaction, session_id, track)?
            .ok_or(RevisionError::MissingSession(session_id))?;
        if session.cursor != expected_current {
            return Err(RevisionError::CursorMoved {
                session: session_id,
                expected: expected_current,
                actual: session.cursor,
            });
        }
        session.cursor = target;
        session.updated_at_ms = now_ms();
        transaction.execute(
            "UPDATE session_cursors SET cursor_revision_id = ?1, updated_at_ms = ?2
             WHERE session_id = ?3 AND track_id = ?4",
            params![
                target.as_bytes().as_slice(),
                session.updated_at_ms,
                session_id.as_bytes().as_slice(),
                track.as_bytes().as_slice()
            ],
        )?;
        update_legacy_default_cursor(
            &transaction,
            session_id,
            track,
            target,
            session.updated_at_ms,
        )?;
        bump_generation(&transaction)?;
        transaction.commit()?;
        self.finish_write()?;
        Ok(session)
    }

    pub fn most_recent_cursor(&self) -> RevisionResult<RevisionId> {
        self.most_recent_cursor_for_track(self.project_info()?.default_track_id)
    }

    pub fn most_recent_cursor_for_track(&self, track: TrackId) -> RevisionResult<RevisionId> {
        most_recent_cursor_for_track_in(&self.connection, track)?.ok_or_else(|| {
            RevisionError::Invalid(format!("track {track} does not have a session cursor"))
        })
    }

    pub fn default_track(&self) -> RevisionResult<Track> {
        let id = self.project_info()?.default_track_id;
        self.track(id)?
            .ok_or_else(|| RevisionError::Invalid(format!("track {id} is missing")))
    }

    pub fn track(&self, id: TrackId) -> RevisionResult<Option<Track>> {
        self.connection
            .query_row(
                "SELECT id, kind, label, root_revision_id, created_at_ms FROM tracks WHERE id = ?1",
                [id.as_bytes().as_slice()],
                track_from_row,
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn tracks(&self) -> RevisionResult<Vec<Track>> {
        let mut statement = self.connection.prepare(
            "SELECT id, kind, label, root_revision_id, created_at_ms
             FROM tracks ORDER BY created_at_ms, id",
        )?;
        let rows = statement.query_map([], track_from_row)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn create_tracks(
        &mut self,
        session_id: SessionId,
        application_version: &str,
        requests: Vec<NewTrack>,
    ) -> RevisionResult<Vec<Track>> {
        self.create_tracks_with_change_set(
            session_id,
            application_version,
            ChangeSetId::new(),
            requests,
        )
    }

    pub fn create_tracks_with_change_set(
        &mut self,
        session_id: SessionId,
        application_version: &str,
        change_set_id: ChangeSetId,
        mut requests: Vec<NewTrack>,
    ) -> RevisionResult<Vec<Track>> {
        if application_version.trim().is_empty() {
            return Err(RevisionError::Invalid(
                "application version is required".into(),
            ));
        }
        for request in &mut requests {
            validate_new_track(request)?;
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let session = session_in(&transaction, session_id)?
            .ok_or(RevisionError::MissingSession(session_id))?;
        let timestamp = now_ms();
        let mut tracks = Vec::with_capacity(requests.len());
        for request in requests {
            let track = Track {
                id: TrackId::new(),
                kind: request.kind,
                label: request.label,
                root_revision: RevisionId::new(),
                created_at_ms: timestamp,
            };
            insert_track(&transaction, &track)?;
            insert_revision(
                &transaction,
                &Revision {
                    id: track.root_revision,
                    track_id: track.id,
                    change_set_id,
                    parent_id: None,
                    actor: session.actor.clone(),
                    session_id,
                    created_at_ms: timestamp,
                    application_version: application_version.into(),
                    label: request.root_label,
                    command_count: 0,
                },
            )?;
            insert_payloads(
                &transaction,
                "snapshots",
                track.root_revision,
                &request.initial_snapshots,
            )?;
            insert_assets(&transaction, &request.assets)?;
            transaction.execute(
                "INSERT INTO session_cursors(
                     session_id, track_id, cursor_revision_id, updated_at_ms
                 ) SELECT id, ?1, ?2, ?3 FROM sessions",
                params![
                    track.id.as_bytes().as_slice(),
                    track.root_revision.as_bytes().as_slice(),
                    timestamp
                ],
            )?;
            tracks.push(track);
        }
        bump_generation(&transaction)?;
        transaction.commit()?;
        self.finish_write()?;
        Ok(tracks)
    }

    pub fn append(&mut self, request: AppendRevision) -> RevisionResult<Revision> {
        Ok(self.append_batch(vec![request])?.remove(0))
    }

    pub fn append_batch(&mut self, requests: Vec<AppendRevision>) -> RevisionResult<Vec<Revision>> {
        self.append_batch_with_change_set(ChangeSetId::new(), requests)
    }

    pub fn append_batch_with_change_set(
        &mut self,
        change_set_id: ChangeSetId,
        mut requests: Vec<AppendRevision>,
    ) -> RevisionResult<Vec<Revision>> {
        if requests.is_empty() {
            return Err(RevisionError::Invalid("revision batch is empty".into()));
        }
        for request in &mut requests {
            validate_append(request)?;
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut revisions = Vec::with_capacity(requests.len());
        for request in requests {
            revisions.push(append_revision_in(&transaction, change_set_id, request)?);
        }
        bump_generation(&transaction)?;
        transaction.commit()?;
        self.finish_write()?;
        Ok(revisions)
    }

    pub fn revisions_for_track(&self, track: TrackId) -> RevisionResult<Vec<Revision>> {
        let mut statement = self.connection.prepare(
            "SELECT id, track_id, change_set_id, parent_id, actor_id, actor_name, actor_kind, session_id,
                    created_at_ms, application_version, label, command_count
             FROM revisions WHERE track_id = ?1 ORDER BY created_at_ms, id",
        )?;
        let rows = statement.query_map([track.as_bytes().as_slice()], revision_from_row)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn sessions_on_track(&self, track: TrackId) -> RevisionResult<Vec<Session>> {
        let mut statement = self.connection.prepare(
            "SELECT s.id, s.actor_id, s.actor_name, s.actor_kind,
                    c.cursor_revision_id, c.updated_at_ms
             FROM sessions s JOIN session_cursors c ON c.session_id = s.id
             WHERE c.track_id = ?1 ORDER BY c.updated_at_ms DESC, s.id",
        )?;
        let rows = statement.query_map([track.as_bytes().as_slice()], session_from_row)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }
}

fn validate_new_track(track: &mut NewTrack) -> RevisionResult<()> {
    if track.kind.trim().is_empty() || track.label.trim().is_empty() {
        return Err(RevisionError::Invalid(
            "track kind and label are required".into(),
        ));
    }
    if track.initial_snapshots.is_empty() {
        return Err(RevisionError::Invalid(
            "a track requires an initial snapshot".into(),
        ));
    }
    for payload in &mut track.initial_snapshots {
        validate_payload(payload)?;
    }
    validate_assets(&track.assets)
}

fn validate_append(request: &mut AppendRevision) -> RevisionResult<()> {
    if request.application_version.trim().is_empty() {
        return Err(RevisionError::Invalid(
            "application version is required".into(),
        ));
    }
    if request.command_count == 0 {
        return Err(RevisionError::Invalid(
            "a non-root revision requires at least one command".into(),
        ));
    }
    if request.operation_payloads.is_empty() {
        return Err(RevisionError::Invalid(
            "a revision requires an operation payload".into(),
        ));
    }
    for payload in request
        .operation_payloads
        .iter_mut()
        .chain(request.snapshots.iter_mut())
    {
        validate_payload(payload)?;
    }
    validate_assets(&request.assets)
}

pub(crate) fn insert_revision(
    transaction: &Transaction<'_>,
    revision: &Revision,
) -> RevisionResult<()> {
    transaction.execute(
        "INSERT INTO revisions(
             id, track_id, change_set_id, parent_id, actor_id, actor_name, actor_kind, session_id,
             created_at_ms, application_version, label, command_count
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            revision.id.as_bytes().as_slice(),
            revision.track_id.as_bytes().as_slice(),
            revision.change_set_id.as_bytes().as_slice(),
            revision.parent_id.map(|id| id.as_bytes().to_vec()),
            revision.actor.id,
            revision.actor.display_name,
            revision.actor.kind.as_str(),
            revision.session_id.as_bytes().as_slice(),
            revision.created_at_ms,
            revision.application_version,
            revision.label,
            revision.command_count
        ],
    )?;
    Ok(())
}

fn append_revision_in(
    transaction: &Transaction<'_>,
    change_set_id: ChangeSetId,
    request: AppendRevision,
) -> RevisionResult<Revision> {
    let session = session_on_track_in(transaction, request.session_id, request.track_id)?
        .ok_or(RevisionError::MissingSession(request.session_id))?;
    if session.cursor != request.expected_parent {
        return Err(RevisionError::CursorMoved {
            session: request.session_id,
            expected: request.expected_parent,
            actual: session.cursor,
        });
    }
    let parent_track = revision_track_in(transaction, request.expected_parent)?
        .ok_or(RevisionError::MissingRevision(request.expected_parent))?;
    if parent_track != request.track_id {
        return Err(RevisionError::Invalid(format!(
            "revision {} does not belong to track {}",
            request.expected_parent, request.track_id
        )));
    }
    let revision = Revision {
        id: RevisionId::new(),
        track_id: request.track_id,
        change_set_id,
        parent_id: Some(request.expected_parent),
        actor: session.actor,
        session_id: request.session_id,
        created_at_ms: now_ms(),
        application_version: request.application_version,
        label: request.label,
        command_count: request.command_count,
    };
    insert_revision(transaction, &revision)?;
    insert_payloads(
        transaction,
        "operation_payloads",
        revision.id,
        &request.operation_payloads,
    )?;
    insert_payloads(transaction, "snapshots", revision.id, &request.snapshots)?;
    insert_assets(transaction, &request.assets)?;
    transaction.execute(
        "UPDATE session_cursors SET cursor_revision_id = ?1, updated_at_ms = ?2
         WHERE session_id = ?3 AND track_id = ?4",
        params![
            revision.id.as_bytes().as_slice(),
            revision.created_at_ms,
            request.session_id.as_bytes().as_slice(),
            request.track_id.as_bytes().as_slice()
        ],
    )?;
    update_legacy_default_cursor(
        transaction,
        request.session_id,
        request.track_id,
        revision.id,
        revision.created_at_ms,
    )?;
    Ok(revision)
}

pub(crate) fn insert_track(transaction: &Transaction<'_>, track: &Track) -> RevisionResult<()> {
    transaction.execute(
        "INSERT INTO tracks(id, kind, label, root_revision_id, created_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            track.id.as_bytes().as_slice(),
            track.kind,
            track.label,
            track.root_revision.as_bytes().as_slice(),
            track.created_at_ms
        ],
    )?;
    Ok(())
}

pub(crate) fn insert_session_cursor(
    transaction: &Transaction<'_>,
    session: SessionId,
    track: TrackId,
    cursor: RevisionId,
    updated_at_ms: i64,
) -> RevisionResult<()> {
    transaction.execute(
        "INSERT INTO session_cursors(
             session_id, track_id, cursor_revision_id, updated_at_ms
         ) VALUES (?1, ?2, ?3, ?4)",
        params![
            session.as_bytes().as_slice(),
            track.as_bytes().as_slice(),
            cursor.as_bytes().as_slice(),
            updated_at_ms
        ],
    )?;
    Ok(())
}

pub(crate) fn update_legacy_default_cursor(
    transaction: &Transaction<'_>,
    session: SessionId,
    track: TrackId,
    cursor: RevisionId,
    updated_at_ms: i64,
) -> RevisionResult<()> {
    if default_track_id_in(transaction)? == track {
        transaction.execute(
            "UPDATE sessions SET cursor_revision_id = ?1, updated_at_ms = ?2 WHERE id = ?3",
            params![
                cursor.as_bytes().as_slice(),
                updated_at_ms,
                session.as_bytes().as_slice()
            ],
        )?;
    } else {
        transaction.execute(
            "UPDATE sessions SET updated_at_ms = ?1 WHERE id = ?2",
            params![updated_at_ms, session.as_bytes().as_slice()],
        )?;
    }
    Ok(())
}

pub(crate) fn session_on_track_in(
    connection: &Connection,
    id: SessionId,
    track: TrackId,
) -> RevisionResult<Option<Session>> {
    connection
        .query_row(
            "SELECT s.actor_id, s.actor_name, s.actor_kind,
                    c.cursor_revision_id, c.updated_at_ms
             FROM sessions s JOIN session_cursors c ON c.session_id = s.id
             WHERE s.id = ?1 AND c.track_id = ?2",
            params![id.as_bytes().as_slice(), track.as_bytes().as_slice()],
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

fn session_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Session> {
    let kind: String = row.get(3)?;
    Ok(Session {
        id: session_id(row.get(0)?)?,
        actor: Actor {
            id: row.get(1)?,
            display_name: row.get(2)?,
            kind: actor_kind(&kind)?,
        },
        cursor: revision_id(row.get(4)?)?,
        updated_at_ms: row.get(5)?,
    })
}

pub(crate) fn most_recent_cursor_for_track_in(
    connection: &Connection,
    track: TrackId,
) -> RevisionResult<Option<RevisionId>> {
    connection
        .query_row(
            "SELECT cursor_revision_id FROM session_cursors
             WHERE track_id = ?1 ORDER BY updated_at_ms DESC, session_id LIMIT 1",
            [track.as_bytes().as_slice()],
            |row| revision_id(row.get(0)?),
        )
        .optional()
        .map_err(Into::into)
}

pub(crate) fn default_track_id_in(connection: &Connection) -> RevisionResult<TrackId> {
    let bytes: Vec<u8> = connection.query_row(
        "SELECT value FROM spectrum_meta WHERE key = 'default_track_id'",
        [],
        |row| row.get(0),
    )?;
    let bytes: [u8; 16] = bytes
        .try_into()
        .map_err(|_| RevisionError::Corrupt("invalid default track id".into()))?;
    Ok(TrackId::from_bytes(bytes))
}

fn revision_track_in(
    connection: &Connection,
    revision: RevisionId,
) -> RevisionResult<Option<TrackId>> {
    connection
        .query_row(
            "SELECT track_id FROM revisions WHERE id = ?1",
            [revision.as_bytes().as_slice()],
            |row| track_id(row.get(0)?),
        )
        .optional()
        .map_err(Into::into)
}

fn track_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Track> {
    Ok(Track {
        id: track_id(row.get(0)?)?,
        kind: row.get(1)?,
        label: row.get(2)?,
        root_revision: revision_id(row.get(3)?)?,
        created_at_ms: row.get(4)?,
    })
}

fn revision_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Revision> {
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

fn session_id(bytes: Vec<u8>) -> rusqlite::Result<SessionId> {
    let bytes: [u8; 16] = bytes
        .try_into()
        .map_err(|_| conversion_error("invalid session id"))?;
    Ok(SessionId::from_bytes(bytes))
}

pub(crate) fn track_id(bytes: Vec<u8>) -> rusqlite::Result<TrackId> {
    let bytes: [u8; 16] = bytes
        .try_into()
        .map_err(|_| conversion_error("invalid track id"))?;
    Ok(TrackId::from_bytes(bytes))
}

fn change_set_id(bytes: Vec<u8>) -> rusqlite::Result<ChangeSetId> {
    let bytes: [u8; 16] = bytes
        .try_into()
        .map_err(|_| conversion_error("invalid change set id"))?;
    Ok(ChangeSetId::from_bytes(bytes))
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
