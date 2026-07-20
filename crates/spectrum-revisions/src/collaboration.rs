use rusqlite::{Connection, OptionalExtension, Row, TransactionBehavior, params};

use crate::{
    Actor, ActorKind, Collaboration, CollaborationMode, CollaborationStatus, CollaborationSync,
    RevisionError, RevisionResult, RevisionStore, SessionId, TrackId,
    metadata::bump_generation,
    storage_io::now_ms,
    store::{revision_id, session_in, validate_actor},
    store_tracks::{session_on_track_in, track_id, update_legacy_default_cursor},
};

impl RevisionStore {
    pub fn start_collaboration(
        &mut self,
        source_session: SessionId,
        track_id: TrackId,
        agent: Actor,
        mode: CollaborationMode,
    ) -> RevisionResult<Collaboration> {
        validate_actor(&agent)?;
        if agent.kind != ActorKind::Agent {
            return Err(RevisionError::Invalid(
                "collaboration participants must use an agent actor".into(),
            ));
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let source = session_on_track_in(&transaction, source_session, track_id)?
            .ok_or(RevisionError::MissingSession(source_session))?;
        let source_default = session_in(&transaction, source_session)?
            .ok_or(RevisionError::MissingSession(source_session))?;
        let agent_session = SessionId::new();
        let timestamp = now_ms();
        if mode == CollaborationMode::Together {
            transaction.execute(
                "UPDATE collaborations
                 SET status = 'superseded', updated_at_ms = ?1
                 WHERE source_session_id = ?2 AND mode = 'together' AND status = 'active'",
                params![timestamp, source_session.as_bytes().as_slice()],
            )?;
        }
        transaction.execute(
            "INSERT INTO sessions(
                 id, actor_id, actor_name, actor_kind, cursor_revision_id, updated_at_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                agent_session.as_bytes().as_slice(),
                agent.id,
                agent.display_name,
                agent.kind.as_str(),
                source_default.cursor.as_bytes().as_slice(),
                timestamp
            ],
        )?;
        transaction.execute(
            "INSERT INTO session_cursors(
                 session_id, track_id, cursor_revision_id, updated_at_ms
             ) SELECT ?1, track_id, cursor_revision_id, ?2
               FROM session_cursors WHERE session_id = ?3",
            params![
                agent_session.as_bytes().as_slice(),
                timestamp,
                source_session.as_bytes().as_slice()
            ],
        )?;
        let collaboration = Collaboration {
            agent_session,
            source_session,
            track_id,
            base_revision: source.cursor,
            followed_revision: source.cursor,
            mode,
            status: CollaborationStatus::Active,
            created_at_ms: timestamp,
            updated_at_ms: timestamp,
        };
        insert_collaboration(&transaction, &collaboration)?;
        bump_generation(&transaction)?;
        transaction.commit()?;
        self.finish_write()?;
        Ok(collaboration)
    }

    pub fn collaboration(&self, agent_session: SessionId) -> RevisionResult<Option<Collaboration>> {
        collaboration_for_agent_in(&self.connection, agent_session)
    }

    pub fn sync_together(
        &mut self,
        source_session: SessionId,
    ) -> RevisionResult<CollaborationSync> {
        if active_together_for_source_in(&self.connection, source_session)?.is_none() {
            return Ok(CollaborationSync::Idle);
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let Some(mut collaboration) = active_together_for_source_in(&transaction, source_session)?
        else {
            return Ok(CollaborationSync::Idle);
        };
        let source = session_on_track_in(&transaction, source_session, collaboration.track_id)?
            .ok_or(RevisionError::MissingSession(source_session))?;
        let agent = session_on_track_in(
            &transaction,
            collaboration.agent_session,
            collaboration.track_id,
        )?
        .ok_or(RevisionError::MissingSession(collaboration.agent_session))?;

        if source.cursor != collaboration.followed_revision {
            let timestamp = now_ms();
            transaction.execute(
                "UPDATE collaborations SET status = 'split', updated_at_ms = ?1
                 WHERE agent_session_id = ?2 AND status = 'active'",
                params![timestamp, collaboration.agent_session.as_bytes().as_slice()],
            )?;
            collaboration.status = CollaborationStatus::Split;
            collaboration.updated_at_ms = timestamp;
            bump_generation(&transaction)?;
            transaction.commit()?;
            self.finish_write()?;
            return Ok(CollaborationSync::Split(collaboration));
        }
        if agent.cursor == collaboration.followed_revision {
            return Ok(CollaborationSync::Waiting(collaboration));
        }

        let from = source.cursor;
        let to = agent.cursor;
        let timestamp = now_ms();
        transaction.execute(
            "UPDATE session_cursors SET cursor_revision_id = ?1, updated_at_ms = ?2
             WHERE session_id = ?3 AND track_id = ?4",
            params![
                to.as_bytes().as_slice(),
                timestamp,
                source_session.as_bytes().as_slice(),
                collaboration.track_id.as_bytes().as_slice()
            ],
        )?;
        update_legacy_default_cursor(
            &transaction,
            source_session,
            collaboration.track_id,
            to,
            timestamp,
        )?;
        transaction.execute(
            "UPDATE collaborations SET followed_revision_id = ?1, updated_at_ms = ?2
             WHERE agent_session_id = ?3 AND status = 'active'",
            params![
                to.as_bytes().as_slice(),
                timestamp,
                collaboration.agent_session.as_bytes().as_slice()
            ],
        )?;
        collaboration.followed_revision = to;
        collaboration.updated_at_ms = timestamp;
        bump_generation(&transaction)?;
        transaction.commit()?;
        self.finish_write()?;
        Ok(CollaborationSync::Advanced {
            collaboration,
            from,
            to,
        })
    }
}

fn insert_collaboration(
    connection: &Connection,
    collaboration: &Collaboration,
) -> RevisionResult<()> {
    connection.execute(
        "INSERT INTO collaborations(
             agent_session_id, source_session_id, track_id, base_revision_id,
             followed_revision_id, mode, status, created_at_ms, updated_at_ms
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            collaboration.agent_session.as_bytes().as_slice(),
            collaboration.source_session.as_bytes().as_slice(),
            collaboration.track_id.as_bytes().as_slice(),
            collaboration.base_revision.as_bytes().as_slice(),
            collaboration.followed_revision.as_bytes().as_slice(),
            collaboration.mode.as_str(),
            collaboration.status.as_str(),
            collaboration.created_at_ms,
            collaboration.updated_at_ms,
        ],
    )?;
    Ok(())
}

fn collaboration_for_agent_in(
    connection: &Connection,
    agent_session: SessionId,
) -> RevisionResult<Option<Collaboration>> {
    connection
        .query_row(
            "SELECT agent_session_id, source_session_id, track_id, base_revision_id,
                    followed_revision_id, mode, status, created_at_ms, updated_at_ms
             FROM collaborations WHERE agent_session_id = ?1",
            [agent_session.as_bytes().as_slice()],
            raw_collaboration,
        )
        .optional()?
        .map(collaboration_from_raw)
        .transpose()
}

fn active_together_for_source_in(
    connection: &Connection,
    source_session: SessionId,
) -> RevisionResult<Option<Collaboration>> {
    connection
        .query_row(
            "SELECT agent_session_id, source_session_id, track_id, base_revision_id,
                    followed_revision_id, mode, status, created_at_ms, updated_at_ms
             FROM collaborations
             WHERE source_session_id = ?1 AND mode = 'together' AND status = 'active'
             ORDER BY updated_at_ms DESC, agent_session_id LIMIT 1",
            [source_session.as_bytes().as_slice()],
            raw_collaboration,
        )
        .optional()?
        .map(collaboration_from_raw)
        .transpose()
}

struct RawCollaboration {
    agent_session: Vec<u8>,
    source_session: Vec<u8>,
    track_id: Vec<u8>,
    base_revision: Vec<u8>,
    followed_revision: Vec<u8>,
    mode: String,
    status: String,
    created_at_ms: i64,
    updated_at_ms: i64,
}

fn raw_collaboration(row: &Row<'_>) -> rusqlite::Result<RawCollaboration> {
    Ok(RawCollaboration {
        agent_session: row.get(0)?,
        source_session: row.get(1)?,
        track_id: row.get(2)?,
        base_revision: row.get(3)?,
        followed_revision: row.get(4)?,
        mode: row.get(5)?,
        status: row.get(6)?,
        created_at_ms: row.get(7)?,
        updated_at_ms: row.get(8)?,
    })
}

fn collaboration_from_raw(raw: RawCollaboration) -> RevisionResult<Collaboration> {
    Ok(Collaboration {
        agent_session: session_id(raw.agent_session)?,
        source_session: session_id(raw.source_session)?,
        track_id: track_id(raw.track_id)?,
        base_revision: revision_id(raw.base_revision)?,
        followed_revision: revision_id(raw.followed_revision)?,
        mode: CollaborationMode::parse(&raw.mode).ok_or_else(|| {
            RevisionError::Corrupt(format!("unknown collaboration mode {}", raw.mode))
        })?,
        status: CollaborationStatus::parse(&raw.status).ok_or_else(|| {
            RevisionError::Corrupt(format!("unknown collaboration status {}", raw.status))
        })?,
        created_at_ms: raw.created_at_ms,
        updated_at_ms: raw.updated_at_ms,
    })
}

fn session_id(bytes: Vec<u8>) -> RevisionResult<SessionId> {
    let bytes: [u8; 16] = bytes
        .try_into()
        .map_err(|_| RevisionError::Corrupt("session id has the wrong length".into()))?;
    Ok(SessionId::from_bytes(bytes))
}
