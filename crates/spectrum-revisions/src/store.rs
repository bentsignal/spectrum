use std::path::{Path, PathBuf};

use rusqlite::{
    Connection, OpenFlags, OptionalExtension, Transaction, TransactionBehavior, params,
};
use sha2::{Digest, Sha256};

use crate::{
    Actor, ActorKind, Asset, AssetId, ChangeSetId, Encoding, NewProject, Payload, Preview,
    ProjectId, ProjectInfo, ReplayPlan, ReplayStep, Revision, RevisionError, RevisionId,
    RevisionResult, Session, SessionId, Track, TrackId,
    metadata::{StorageStateId, bump_generation, generation_in, state_id_in, write_meta},
    schema::{self, CONTAINER_FORMAT},
    storage_io::{now_ms, sidecar_path, sync_if_present},
    store_tracks::{
        default_track_id_in, insert_revision, insert_session_cursor, insert_track,
        most_recent_cursor_for_track_in, track_id,
    },
};

pub trait Compatibility {
    fn supports_snapshot(&self, encoding: &Encoding) -> bool;
    fn supports_operations(&self, encoding: &Encoding) -> bool;
}

pub struct RevisionStore {
    pub(crate) connection: Connection,
    path: PathBuf,
}

pub(crate) struct StoreInspection {
    pub info: ProjectInfo,
    pub generation: u64,
    pub state_id: Option<StorageStateId>,
}

impl RevisionStore {
    pub fn create(path: &Path, mut project: NewProject) -> RevisionResult<(Self, ProjectInfo)> {
        validate_project(&mut project)?;
        let connection = Connection::open(path)?;
        schema::configure(&connection)?;
        schema::initialize(&connection)?;
        let mut store = Self {
            connection,
            path: path.to_owned(),
        };
        if store.meta("project_id")?.is_some() {
            return Err(RevisionError::AlreadyInitialized);
        }

        let project_id = ProjectId::new();
        let default_track_id = TrackId::new();
        let root_revision = RevisionId::new();
        let created_at_ms = now_ms();
        let transaction = store
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        write_meta(&transaction, "project_id", project_id.as_bytes())?;
        write_meta(
            &transaction,
            "application_id",
            project.application_id.as_bytes(),
        )?;
        write_meta(
            &transaction,
            "created_at_ms",
            created_at_ms.to_string().as_bytes(),
        )?;
        write_meta(&transaction, "root_revision", root_revision.as_bytes())?;
        write_meta(
            &transaction,
            "default_track_id",
            default_track_id.as_bytes(),
        )?;
        write_meta(&transaction, "storage_generation", b"1")?;
        write_meta(
            &transaction,
            "storage_state_id",
            SessionId::new().as_bytes(),
        )?;
        insert_track(
            &transaction,
            &Track {
                id: default_track_id,
                kind: project.track_kind,
                label: project.track_label,
                root_revision,
                created_at_ms,
            },
        )?;
        insert_revision(
            &transaction,
            &Revision {
                id: root_revision,
                track_id: default_track_id,
                change_set_id: ChangeSetId::new(),
                parent_id: None,
                actor: project.actor.clone(),
                session_id: project.session_id,
                created_at_ms,
                application_version: project.application_version,
                label: project.root_label,
                command_count: 0,
            },
        )?;
        insert_payloads(
            &transaction,
            "snapshots",
            root_revision,
            &project.initial_snapshots,
        )?;
        insert_assets(&transaction, &project.assets)?;
        insert_session(
            &transaction,
            project.session_id,
            &project.actor,
            root_revision,
            created_at_ms,
        )?;
        transaction.commit()?;
        store.finish_write()?;

        let info = ProjectInfo {
            project_id,
            application_id: project.application_id,
            created_at_ms,
            container_format: CONTAINER_FORMAT,
            default_track_id,
            root_revision,
        };
        Ok((store, info))
    }

    pub fn open(path: &Path) -> RevisionResult<Self> {
        let mut connection = Connection::open(path)?;
        schema::configure(&connection)?;
        schema::verify_header(&connection)?;
        schema::migrate(&mut connection)?;
        let store = Self {
            connection,
            path: path.to_owned(),
        };
        if store.meta("project_id")?.is_none() {
            return Err(RevisionError::NotARevisionStore);
        }
        Ok(store)
    }

    pub fn project_info(&self) -> RevisionResult<ProjectInfo> {
        project_info_in(&self.connection)
    }

    pub fn generation(&self) -> RevisionResult<u64> {
        generation_in(&self.connection)
    }

    pub(crate) fn state_id(&self) -> RevisionResult<Option<StorageStateId>> {
        state_id_in(&self.connection)
    }

    pub(crate) fn inspect(path: &Path) -> RevisionResult<StoreInspection> {
        let uri = url::Url::from_file_path(path)
            .map_err(|_| RevisionError::Invalid("project path is not absolute".into()))?;
        let uri = format!("{uri}?immutable=1");
        let connection = Connection::open_with_flags(
            uri,
            OpenFlags::SQLITE_OPEN_READ_ONLY
                | OpenFlags::SQLITE_OPEN_URI
                | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        schema::verify_header(&connection)?;
        Ok(StoreInspection {
            info: project_info_in(&connection)?,
            generation: generation_in(&connection)?,
            state_id: state_id_in(&connection)?,
        })
    }

    pub fn resume_session(
        &mut self,
        id: SessionId,
        actor: Actor,
        fallback: RevisionId,
    ) -> RevisionResult<Session> {
        validate_actor(&actor)?;
        self.require_revision(fallback)?;
        if let Some(session) = self.session(id)? {
            let default_track = self.project_info()?.default_track_id;
            if self.session_on_track(id, default_track)?.is_none() {
                let transaction = self
                    .connection
                    .transaction_with_behavior(TransactionBehavior::Immediate)?;
                insert_session_cursor(
                    &transaction,
                    id,
                    default_track,
                    session.cursor,
                    session.updated_at_ms,
                )?;
                bump_generation(&transaction)?;
                transaction.commit()?;
                self.finish_write()?;
            }
            return Ok(session);
        }
        let updated_at_ms = now_ms();
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        insert_session(&transaction, id, &actor, fallback, updated_at_ms)?;
        let default_track = default_track_id_in(&transaction)?;
        let mut statement = transaction.prepare(
            "SELECT id, root_revision_id FROM tracks WHERE id != ?1 ORDER BY created_at_ms, id",
        )?;
        let tracks = statement
            .query_map([default_track.as_bytes().as_slice()], |row| {
                Ok((track_id(row.get(0)?)?, revision_id(row.get(1)?)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        drop(statement);
        for (track, root) in tracks {
            let cursor = most_recent_cursor_for_track_in(&transaction, track)?.unwrap_or(root);
            insert_session_cursor(&transaction, id, track, cursor, updated_at_ms)?;
        }
        bump_generation(&transaction)?;
        transaction.commit()?;
        self.finish_write()?;
        Ok(Session {
            id,
            actor,
            cursor: fallback,
            updated_at_ms,
        })
    }

    pub fn session(&self, id: SessionId) -> RevisionResult<Option<Session>> {
        self.connection
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

    pub fn remember_child(
        &mut self,
        session_id: SessionId,
        parent: RevisionId,
        child: RevisionId,
    ) -> RevisionResult<()> {
        let child_revision = self
            .revision(child)?
            .ok_or(RevisionError::MissingRevision(child))?;
        if child_revision.parent_id != Some(parent) {
            return Err(RevisionError::Invalid(format!(
                "revision {child} is not a child of {parent}"
            )));
        }
        if self.session(session_id)?.is_none() {
            return Err(RevisionError::MissingSession(session_id));
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            "INSERT INTO session_child_choices(
                 session_id, parent_revision_id, child_revision_id
             ) VALUES (?1, ?2, ?3)
             ON CONFLICT(session_id, parent_revision_id)
             DO UPDATE SET child_revision_id = excluded.child_revision_id",
            params![
                session_id.as_bytes().as_slice(),
                parent.as_bytes().as_slice(),
                child.as_bytes().as_slice()
            ],
        )?;
        bump_generation(&transaction)?;
        transaction.commit()?;
        self.finish_write()?;
        Ok(())
    }

    pub fn preferred_child(
        &self,
        session_id: SessionId,
        parent: RevisionId,
    ) -> RevisionResult<Option<RevisionId>> {
        self.connection
            .query_row(
                "SELECT child_revision_id FROM session_child_choices
                 WHERE session_id = ?1 AND parent_revision_id = ?2",
                params![
                    session_id.as_bytes().as_slice(),
                    parent.as_bytes().as_slice()
                ],
                |row| revision_id(row.get(0)?),
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn revision(&self, id: RevisionId) -> RevisionResult<Option<Revision>> {
        revision_in(&self.connection, id)
    }

    pub fn children(&self, parent: RevisionId) -> RevisionResult<Vec<Revision>> {
        self.require_revision(parent)?;
        let mut statement = self.connection.prepare(
            "SELECT id, track_id, change_set_id, parent_id, actor_id, actor_name, actor_kind, session_id,
                    created_at_ms, application_version, label, command_count
             FROM revisions WHERE parent_id = ?1 ORDER BY created_at_ms, id",
        )?;
        let rows = statement.query_map([parent.as_bytes().as_slice()], revision_from_row)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn revisions(&self) -> RevisionResult<Vec<Revision>> {
        let mut statement = self.connection.prepare(
            "SELECT id, track_id, change_set_id, parent_id, actor_id, actor_name, actor_kind, session_id,
                    created_at_ms, application_version, label, command_count
             FROM revisions ORDER BY created_at_ms, id",
        )?;
        let rows = statement.query_map([], revision_from_row)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn sessions(&self) -> RevisionResult<Vec<Session>> {
        let mut statement = self.connection.prepare(
            "SELECT id, actor_id, actor_name, actor_kind, cursor_revision_id, updated_at_ms
             FROM sessions ORDER BY updated_at_ms DESC, id",
        )?;
        let rows = statement.query_map([], |row| {
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
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn ancestry(&self, target: RevisionId) -> RevisionResult<Vec<Revision>> {
        let mut ancestry = Vec::new();
        let mut current = target;
        loop {
            let revision = self
                .revision(current)?
                .ok_or(RevisionError::MissingRevision(current))?;
            let parent = revision.parent_id;
            ancestry.push(revision);
            let Some(parent) = parent else { break };
            current = parent;
        }
        ancestry.reverse();
        Ok(ancestry)
    }

    pub fn replay_plan(
        &self,
        target: RevisionId,
        compatibility: &impl Compatibility,
    ) -> RevisionResult<ReplayPlan> {
        let ancestry = self.ancestry(target)?;
        for snapshot_index in (0..ancestry.len()).rev() {
            let snapshot_revision = ancestry[snapshot_index].id;
            let Some(snapshot) = self.best_payload("snapshots", snapshot_revision, |encoding| {
                compatibility.supports_snapshot(encoding)
            })?
            else {
                continue;
            };
            let mut steps = Vec::new();
            let mut compatible = true;
            for revision in &ancestry[snapshot_index + 1..] {
                let Some(operations) =
                    self.best_payload("operation_payloads", revision.id, |encoding| {
                        compatibility.supports_operations(encoding)
                    })?
                else {
                    compatible = false;
                    break;
                };
                steps.push(ReplayStep {
                    revision: revision.clone(),
                    operations,
                });
            }
            if compatible {
                return Ok(ReplayPlan {
                    target,
                    snapshot_revision,
                    snapshot,
                    steps,
                });
            }
        }
        Err(RevisionError::IncompatibleRevision(target))
    }

    pub fn newest_compatible_ancestor(
        &self,
        target: RevisionId,
        compatibility: &impl Compatibility,
    ) -> RevisionResult<RevisionId> {
        for revision in self.ancestry(target)?.into_iter().rev() {
            match self.replay_plan(revision.id, compatibility) {
                Ok(_) => return Ok(revision.id),
                Err(RevisionError::IncompatibleRevision(_)) => {}
                Err(error) => return Err(error),
            }
        }
        Err(RevisionError::IncompatibleRevision(target))
    }

    pub fn add_snapshot(
        &mut self,
        revision: RevisionId,
        mut payload: Payload,
    ) -> RevisionResult<()> {
        validate_payload(&mut payload)?;
        self.require_revision(revision)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        insert_payloads(&transaction, "snapshots", revision, &[payload])?;
        bump_generation(&transaction)?;
        transaction.commit()?;
        self.finish_write()?;
        Ok(())
    }

    pub fn put_asset(&mut self, media_type: &str, bytes: &[u8]) -> RevisionResult<AssetId> {
        if media_type.trim().is_empty() {
            return Err(RevisionError::Invalid("asset media type is empty".into()));
        }
        let asset = Asset::new(media_type, bytes);
        let id = asset.id;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        insert_assets(&transaction, &[asset])?;
        bump_generation(&transaction)?;
        transaction.commit()?;
        self.finish_write()?;
        Ok(id)
    }

    pub fn asset(&self, id: AssetId) -> RevisionResult<Option<Vec<u8>>> {
        self.connection
            .query_row(
                "SELECT bytes FROM assets WHERE sha256 = ?1",
                [id.as_bytes().as_slice()],
                |row| row.get(0),
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn asset_record(&self, id: AssetId) -> RevisionResult<Option<Asset>> {
        self.connection
            .query_row(
                "SELECT media_type, bytes FROM assets WHERE sha256 = ?1",
                [id.as_bytes().as_slice()],
                |row| {
                    Ok(Asset {
                        id,
                        media_type: row.get(0)?,
                        bytes: row.get(1)?,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn put_preview(&mut self, preview: &Preview) -> RevisionResult<()> {
        self.require_revision(preview.revision_id)?;
        if preview.width == 0 || preview.height == 0 || preview.format.trim().is_empty() {
            return Err(RevisionError::Invalid("preview metadata is invalid".into()));
        }
        let hash = Sha256::digest(&preview.bytes);
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            "INSERT OR REPLACE INTO previews(
                 revision_id, format, width, height, bytes, sha256
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                preview.revision_id.as_bytes().as_slice(),
                preview.format,
                preview.width,
                preview.height,
                preview.bytes,
                hash.as_slice()
            ],
        )?;
        bump_generation(&transaction)?;
        transaction.commit()?;
        self.finish_write()?;
        Ok(())
    }

    pub fn verify_integrity(&self) -> RevisionResult<()> {
        let check: String = self
            .connection
            .query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
        if check != "ok" {
            return Err(RevisionError::Corrupt(check));
        }
        let invalid_track_links: bool = self.connection.query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM revisions r
                 LEFT JOIN tracks t ON t.id = r.track_id
                 LEFT JOIN revisions p ON p.id = r.parent_id
                 WHERE t.id IS NULL OR (r.parent_id IS NOT NULL AND p.track_id != r.track_id)
                 UNION ALL
                 SELECT 1 FROM session_cursors c
                 LEFT JOIN revisions r ON r.id = c.cursor_revision_id
                 WHERE r.id IS NULL OR r.track_id != c.track_id
             )",
            [],
            |row| row.get(0),
        )?;
        if invalid_track_links {
            return Err(RevisionError::Corrupt(
                "revision track or session cursor relationship is invalid".into(),
            ));
        }
        for table in ["operation_payloads", "snapshots", "previews"] {
            let sql = format!("SELECT bytes, sha256 FROM {table}");
            let mut statement = self.connection.prepare(&sql)?;
            let rows = statement.query_map([], |row| {
                Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, Vec<u8>>(1)?))
            })?;
            for row in rows {
                let (bytes, stored_hash) = row?;
                if Sha256::digest(&bytes).as_slice() != stored_hash {
                    return Err(RevisionError::Corrupt(format!(
                        "payload hash mismatch in {table}"
                    )));
                }
            }
        }
        let mut statement = self
            .connection
            .prepare("SELECT bytes, sha256, byte_length FROM assets")?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, Vec<u8>>(0)?,
                row.get::<_, Vec<u8>>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })?;
        for row in rows {
            let (bytes, stored_hash, byte_length) = row?;
            if bytes.len() as i64 != byte_length || Sha256::digest(&bytes).as_slice() != stored_hash
            {
                return Err(RevisionError::Corrupt("asset hash mismatch".into()));
            }
        }
        Ok(())
    }

    pub fn checkpoint(&self) -> RevisionResult<()> {
        self.finish_write()
    }

    pub(crate) fn finish_write(&self) -> RevisionResult<()> {
        self.connection
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")?;
        sync_if_present(&self.path)?;
        sync_if_present(&sidecar_path(&self.path, "-wal"))?;
        Ok(())
    }

    fn best_payload(
        &self,
        table: &str,
        revision: RevisionId,
        supports: impl Fn(&Encoding) -> bool,
    ) -> RevisionResult<Option<Payload>> {
        let sql = format!(
            "SELECT family, version, capabilities_json, bytes, sha256
             FROM {table} WHERE revision_id = ?1 ORDER BY version DESC, family"
        );
        let mut statement = self.connection.prepare(&sql)?;
        let rows = statement.query_map([revision.as_bytes().as_slice()], |row| {
            let family: String = row.get(0)?;
            let version: u32 = row.get(1)?;
            let capabilities_json: String = row.get(2)?;
            let bytes: Vec<u8> = row.get(3)?;
            let hash: Vec<u8> = row.get(4)?;
            Ok((family, version, capabilities_json, bytes, hash))
        })?;
        for row in rows {
            let (family, version, capabilities_json, bytes, hash) = row?;
            if Sha256::digest(&bytes).as_slice() != hash {
                return Err(RevisionError::Corrupt(format!(
                    "payload hash mismatch in {table}"
                )));
            }
            let encoding = Encoding {
                family,
                version,
                required_capabilities: serde_json::from_str(&capabilities_json)?,
            };
            if supports(&encoding) {
                return Ok(Some(Payload { encoding, bytes }));
            }
        }
        Ok(None)
    }

    fn require_revision(&self, id: RevisionId) -> RevisionResult<()> {
        require_revision_in(&self.connection, id)
    }

    fn meta(&self, key: &str) -> RevisionResult<Option<Vec<u8>>> {
        self.connection
            .query_row(
                "SELECT value FROM spectrum_meta WHERE key = ?1",
                [key],
                |row| row.get(0),
            )
            .optional()
            .map_err(Into::into)
    }
}

fn validate_project(project: &mut NewProject) -> RevisionResult<()> {
    if project.application_id.trim().is_empty() || project.application_version.trim().is_empty() {
        return Err(RevisionError::Invalid(
            "application id and version are required".into(),
        ));
    }
    validate_actor(&project.actor)?;
    if project.track_kind.trim().is_empty() || project.track_label.trim().is_empty() {
        return Err(RevisionError::Invalid(
            "initial track kind and label are required".into(),
        ));
    }
    if project.initial_snapshots.is_empty() {
        return Err(RevisionError::Invalid(
            "a project requires an initial snapshot".into(),
        ));
    }
    for payload in &mut project.initial_snapshots {
        validate_payload(payload)?;
    }
    validate_assets(&project.assets)?;
    Ok(())
}

pub(crate) fn validate_actor(actor: &Actor) -> RevisionResult<()> {
    if actor.id.trim().is_empty() || actor.display_name.trim().is_empty() {
        return Err(RevisionError::Invalid(
            "actor id and display name are required".into(),
        ));
    }
    Ok(())
}

pub(crate) fn validate_payload(payload: &mut Payload) -> RevisionResult<()> {
    payload.encoding.normalize();
    if payload.encoding.family.trim().is_empty() {
        return Err(RevisionError::Invalid(
            "payload encoding family is empty".into(),
        ));
    }
    if payload
        .encoding
        .required_capabilities
        .iter()
        .any(|capability| capability.trim().is_empty())
    {
        return Err(RevisionError::Invalid("payload capability is empty".into()));
    }
    Ok(())
}

pub(crate) fn insert_payloads(
    transaction: &Transaction<'_>,
    table: &str,
    revision: RevisionId,
    payloads: &[Payload],
) -> RevisionResult<()> {
    let sql = format!(
        "INSERT INTO {table}(
             revision_id, family, version, capabilities_json, bytes, sha256
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)"
    );
    for payload in payloads {
        let capabilities = serde_json::to_string(&payload.encoding.required_capabilities)?;
        let hash = Sha256::digest(&payload.bytes);
        transaction.execute(
            &sql,
            params![
                revision.as_bytes().as_slice(),
                payload.encoding.family,
                payload.encoding.version,
                capabilities,
                payload.bytes,
                hash.as_slice()
            ],
        )?;
    }
    Ok(())
}

pub(crate) fn insert_session(
    transaction: &Transaction<'_>,
    id: SessionId,
    actor: &Actor,
    cursor: RevisionId,
    updated_at_ms: i64,
) -> RevisionResult<()> {
    transaction.execute(
        "INSERT INTO sessions(
             id, actor_id, actor_name, actor_kind, cursor_revision_id, updated_at_ms
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            id.as_bytes().as_slice(),
            actor.id,
            actor.display_name,
            actor.kind.as_str(),
            cursor.as_bytes().as_slice(),
            updated_at_ms
        ],
    )?;
    let default_track = default_track_id_in(transaction)?;
    insert_session_cursor(transaction, id, default_track, cursor, updated_at_ms)?;
    Ok(())
}

pub(crate) fn insert_assets(transaction: &Transaction<'_>, assets: &[Asset]) -> RevisionResult<()> {
    for asset in assets {
        transaction.execute(
            "INSERT OR IGNORE INTO assets(sha256, media_type, byte_length, bytes)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                asset.id.as_bytes().as_slice(),
                asset.media_type,
                asset.bytes.len() as i64,
                asset.bytes
            ],
        )?;
    }
    Ok(())
}

pub(crate) fn validate_assets(assets: &[Asset]) -> RevisionResult<()> {
    for asset in assets {
        if asset.media_type.trim().is_empty() {
            return Err(RevisionError::Invalid("asset media type is empty".into()));
        }
        if AssetId::for_bytes(&asset.bytes) != asset.id {
            return Err(RevisionError::Invalid(format!(
                "asset {} does not match its content hash",
                asset.id
            )));
        }
    }
    Ok(())
}

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

fn revision_in(connection: &Connection, id: RevisionId) -> RevisionResult<Option<Revision>> {
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

fn require_revision_in(connection: &Connection, id: RevisionId) -> RevisionResult<()> {
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

fn project_info_in(connection: &Connection) -> RevisionResult<ProjectInfo> {
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

fn session_id(bytes: Vec<u8>) -> rusqlite::Result<SessionId> {
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
