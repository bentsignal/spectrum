mod codec;

use std::{
    collections::HashMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use codec::{
    AssetReference, CATALOG_TRACK_KIND, CatalogCompatibility, CatalogState, LegacyCompatibility,
    PHOTO_TRACK_KIND, PhotoCompatibility, PreparedPhoto, catalog_operation, catalog_snapshot,
    decode_catalog_operation, decode_catalog_snapshot, decode_legacy_snapshot,
    decode_photo_operation, decode_photo_snapshot, photo_id_from_track_label, photo_operation,
    photo_snapshot, photo_track_label,
};
use serde::Serialize;
use spectrum_revisions::{
    Actor, ActorKind, AppendRevision, ChangeSetId, Collaboration, CollaborationMode,
    CollaborationSync, LiveRevisionStore, NewProject, NewTrack, ProjectInfo, PublishStats,
    Revision, RevisionId, Session, SessionId, TrackId,
};

use crate::{Command, Photo, Project, command::apply_replay_command};

const APPLICATION_ID: &str = "spectrum.lumen";
const SNAPSHOT_COMMAND_BUDGET: u64 = 100;
const SNAPSHOT_OPERATION_BYTE_BUDGET: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, Default)]
struct SnapshotTail {
    commands: u64,
    operation_bytes: usize,
}

impl SnapshotTail {
    fn after(self, command_count: usize, operation_bytes: usize) -> Self {
        Self {
            commands: self
                .commands
                .saturating_add(command_count.try_into().unwrap_or(u64::MAX)),
            operation_bytes: self.operation_bytes.saturating_add(operation_bytes),
        }
    }

    fn needs_snapshot(self) -> bool {
        self.commands >= SNAPSHOT_COMMAND_BUDGET
            || self.operation_bytes >= SNAPSHOT_OPERATION_BYTE_BUDGET
    }
}

pub struct DurableCatalog {
    store: LiveRevisionStore,
    info: ProjectInfo,
    actor: Actor,
    session_id: SessionId,
    catalog_cursor: RevisionId,
    photo_tracks: HashMap<u64, TrackId>,
    photo_cursors: HashMap<u64, RevisionId>,
    photo_assets: HashMap<u64, AssetReference>,
    tails: HashMap<TrackId, SnapshotTail>,
    restricted_track: Option<TrackId>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ProjectHistory {
    pub photo_id: u64,
    pub track_id: TrackId,
    pub root: RevisionId,
    pub current: RevisionId,
    pub revisions: Vec<Revision>,
    pub sessions: Vec<Session>,
}

impl DurableCatalog {
    pub fn looks_durable(path: &Path) -> Result<bool> {
        let mut file = fs::File::open(path)?;
        let mut header = [0; 16];
        if file.read_exact(&mut header).is_err() {
            return Ok(false);
        }
        Ok(&header == b"SQLite format 3\0")
    }

    pub fn create(
        path: &Path,
        project: &Project,
        actor: Actor,
        session_id: SessionId,
    ) -> Result<(Self, Project)> {
        if path.exists() && path.metadata()?.len() > 0 {
            bail!("refusing to replace existing project {}", path.display());
        }
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent)?;
        }
        let catalog_state = CatalogState::from_project(project);
        let project_actor = actor.clone();
        let (mut store, info) = LiveRevisionStore::create(
            path,
            &live_cache_root(path)?,
            NewProject {
                application_id: APPLICATION_ID.into(),
                application_version: env!("CARGO_PKG_VERSION").into(),
                actor,
                session_id,
                root_label: Some("Created catalog".into()),
                track_kind: CATALOG_TRACK_KIND.into(),
                track_label: "Catalog".into(),
                initial_snapshots: vec![catalog_snapshot(&catalog_state)?],
                assets: Vec::new(),
            },
        )?;
        let prepared = prepare_new_photos(&project.photos)?;
        let new_tracks = prepared
            .iter()
            .map(|(photo, prepared)| new_photo_track(photo, prepared))
            .collect::<Result<Vec<_>>>()?;
        let tracks = if new_tracks.is_empty() {
            Vec::new()
        } else {
            store.mutate(|store| {
                store.create_tracks(session_id, env!("CARGO_PKG_VERSION"), new_tracks)
            })?
        };
        let mut catalog = Self::empty(store, info, project_actor, session_id);
        catalog.catalog_cursor = catalog.info.root_revision;
        for ((photo, prepared), track) in prepared.into_iter().zip(tracks) {
            catalog.photo_tracks.insert(photo.id, track.id);
            catalog.photo_cursors.insert(photo.id, track.root_revision);
            catalog.photo_assets.insert(photo.id, prepared.reference);
            catalog.tails.insert(track.id, SnapshotTail::default());
        }
        Ok((catalog, project.clone()))
    }

    pub fn open(path: &Path, actor: Actor, session_id: SessionId) -> Result<(Self, Project)> {
        let mut store = LiveRevisionStore::open(path, &live_cache_root(path)?)?;
        let info = checked_project_info(&store, path)?;
        upgrade_legacy_if_needed(&mut store, &info, session_id)?;
        let latest = store
            .store()
            .most_recent_cursor_for_track(info.default_track_id)?;
        let fallback = store
            .store()
            .newest_compatible_ancestor(latest, &CatalogCompatibility)?;
        store.mutate(|store| store.resume_session(session_id, actor.clone(), fallback))?;
        let catalog_session = store
            .store()
            .session_on_track(session_id, info.default_track_id)?
            .context("Lumen session has no catalog cursor")?;
        let catalog_cursor = store
            .store()
            .newest_compatible_ancestor(catalog_session.cursor, &CatalogCompatibility)?;
        if catalog_cursor != catalog_session.cursor {
            store.mutate(|store| {
                store.move_session(session_id, catalog_session.cursor, catalog_cursor)
            })?;
        }
        let mut catalog = Self::empty(store, info, actor, session_id);
        catalog.catalog_cursor = catalog_cursor;
        let project = catalog.load_session_project(None)?;
        Ok((catalog, project))
    }

    pub fn open_session(path: &Path, session_id: SessionId) -> Result<(Self, Project)> {
        let mut store = LiveRevisionStore::open(path, &live_cache_root(path)?)?;
        let info = checked_project_info(&store, path)?;
        upgrade_legacy_if_needed(&mut store, &info, session_id)?;
        let session = store
            .store()
            .session_on_track(session_id, info.default_track_id)?
            .with_context(|| format!("agent session {session_id} does not exist"))?;
        let catalog_cursor = store
            .store()
            .newest_compatible_ancestor(session.cursor, &CatalogCompatibility)?;
        if catalog_cursor != session.cursor {
            store.mutate(|store| store.move_session(session_id, session.cursor, catalog_cursor))?;
        }
        let restricted_track = store
            .store()
            .collaboration(session_id)?
            .map(|collaboration| collaboration.track_id);
        let mut catalog = Self::empty(store, info, session.actor, session_id);
        catalog.catalog_cursor = catalog_cursor;
        catalog.restricted_track = restricted_track;
        let project = catalog.load_session_project(None)?;
        Ok((catalog, project))
    }

    fn empty(
        store: LiveRevisionStore,
        info: ProjectInfo,
        actor: Actor,
        session_id: SessionId,
    ) -> Self {
        let catalog_cursor = info.root_revision;
        Self {
            store,
            info,
            actor,
            session_id,
            catalog_cursor,
            photo_tracks: HashMap::new(),
            photo_cursors: HashMap::new(),
            photo_assets: HashMap::new(),
            tails: HashMap::new(),
            restricted_track: None,
        }
    }

    pub fn start_collaboration(
        path: &Path,
        source_session: Option<SessionId>,
        photo_id: u64,
        agent: Actor,
        mode: CollaborationMode,
    ) -> Result<Collaboration> {
        let mut store = LiveRevisionStore::open(path, &live_cache_root(path)?)?;
        let info = checked_project_info(&store, path)?;
        let track = photo_track(&store, photo_id)?;
        let sessions = store.store().sessions_on_track(track)?;
        let source_session = source_session_from(&sessions, source_session)?;
        let collaboration =
            store.mutate(|store| store.start_collaboration(source_session, track, agent, mode))?;
        debug_assert_eq!(info.application_id, APPLICATION_ID);
        Ok(collaboration)
    }

    pub fn collaboration(path: &Path, agent_session: SessionId) -> Result<Collaboration> {
        let store = LiveRevisionStore::open(path, &live_cache_root(path)?)?;
        checked_project_info(&store, path)?;
        store
            .store()
            .collaboration(agent_session)?
            .with_context(|| format!("agent session {agent_session} is not a collaboration"))
    }

    pub fn load_current(path: &Path) -> Result<Project> {
        let store = LiveRevisionStore::open(path, &live_cache_root(path)?)?;
        let info = checked_project_info(&store, path)?;
        let catalog_cursor = store
            .store()
            .most_recent_cursor_for_track(info.default_track_id)?;
        let (catalog_state, _) = load_catalog(&store, catalog_cursor)?;
        let tracks = photo_track_map(&store)?;
        let mut photos = Vec::with_capacity(catalog_state.photo_ids().len());
        for id in catalog_state.photo_ids() {
            let track = *tracks
                .get(id)
                .with_context(|| format!("photo {id} is missing its revision track"))?;
            let cursor = store.store().most_recent_cursor_for_track(track)?;
            photos.push(load_photo(&store, &info, cursor)?.0);
        }
        Ok(catalog_state.assemble(photos, None))
    }

    pub fn commit(
        &mut self,
        commands: &[Command],
        before: &Project,
        project: &Project,
        label: impl Into<String>,
    ) -> Result<Vec<RevisionId>> {
        if commands.is_empty() {
            bail!("cannot commit an empty Lumen action");
        }
        let label = label.into();
        let before_by_id: HashMap<_, _> = before
            .photos
            .iter()
            .map(|photo| (photo.id, photo))
            .collect();
        let new_photos: Vec<_> = project
            .photos
            .iter()
            .filter(|photo| !before_by_id.contains_key(&photo.id))
            .cloned()
            .collect();
        let prepared = prepare_new_photos(&new_photos)?;
        let new_track_requests = prepared
            .iter()
            .map(|(photo, prepared)| new_photo_track(photo, prepared))
            .collect::<Result<Vec<_>>>()?;
        let mut requests = Vec::new();
        let mut next_tails = HashMap::new();
        for photo in &project.photos {
            let Some(previous) = before_by_id.get(&photo.id) else {
                continue;
            };
            if *previous == photo {
                continue;
            }
            let track = self.track_for_photo(photo.id)?;
            self.require_allowed_track(track)?;
            let reference = self
                .photo_assets
                .get(&photo.id)
                .context("photo asset reference is missing")?;
            let operation = photo_operation(photo, reference)?;
            let next_tail = self
                .tails
                .get(&track)
                .copied()
                .unwrap_or_default()
                .after(1, operation.bytes.len());
            let snapshots = if next_tail.needs_snapshot() {
                vec![photo_snapshot(photo, reference)?]
            } else {
                Vec::new()
            };
            requests.push(AppendRevision {
                track_id: track,
                session_id: self.session_id,
                expected_parent: self.photo_cursor(photo.id)?,
                application_version: env!("CARGO_PKG_VERSION").into(),
                label: Some(label.clone()),
                command_count: 1,
                operation_payloads: vec![operation],
                snapshots,
                assets: Vec::new(),
            });
            next_tails.insert(
                track,
                if next_tail.needs_snapshot() {
                    SnapshotTail::default()
                } else {
                    next_tail
                },
            );
        }
        let before_catalog = CatalogState::from_project(before);
        let next_catalog = CatalogState::from_project(project);
        if before_catalog != next_catalog {
            self.require_allowed_track(self.info.default_track_id)?;
            let operation = catalog_operation(&next_catalog)?;
            let next_tail = self
                .tails
                .get(&self.info.default_track_id)
                .copied()
                .unwrap_or_default()
                .after(1, operation.bytes.len());
            let snapshots = if next_tail.needs_snapshot() {
                vec![catalog_snapshot(&next_catalog)?]
            } else {
                Vec::new()
            };
            requests.push(AppendRevision {
                track_id: self.info.default_track_id,
                session_id: self.session_id,
                expected_parent: self.catalog_cursor,
                application_version: env!("CARGO_PKG_VERSION").into(),
                label: Some(label.clone()),
                command_count: 1,
                operation_payloads: vec![operation],
                snapshots,
                assets: Vec::new(),
            });
            next_tails.insert(
                self.info.default_track_id,
                if next_tail.needs_snapshot() {
                    SnapshotTail::default()
                } else {
                    next_tail
                },
            );
        }
        if requests.is_empty() && new_track_requests.is_empty() {
            bail!("Lumen action did not change a durable track");
        }
        let change_set_id = ChangeSetId::new();
        let (tracks, revisions) = self.store.mutate(|store| {
            let tracks = if new_track_requests.is_empty() {
                Vec::new()
            } else {
                store.create_tracks_with_change_set(
                    self.session_id,
                    env!("CARGO_PKG_VERSION"),
                    change_set_id,
                    new_track_requests,
                )?
            };
            let revisions = if requests.is_empty() {
                Vec::new()
            } else {
                store.append_batch_with_change_set(change_set_id, requests)?
            };
            Ok((tracks, revisions))
        })?;
        for ((photo, prepared), track) in prepared.into_iter().zip(tracks) {
            self.photo_tracks.insert(photo.id, track.id);
            self.photo_cursors.insert(photo.id, track.root_revision);
            self.photo_assets.insert(photo.id, prepared.reference);
            self.tails.insert(track.id, SnapshotTail::default());
        }
        let mut ids = Vec::with_capacity(revisions.len());
        for revision in revisions {
            if revision.track_id == self.info.default_track_id {
                self.catalog_cursor = revision.id;
            } else if let Some(photo_id) = self.photo_id_for_track(revision.track_id) {
                self.photo_cursors.insert(photo_id, revision.id);
            }
            if let Some(tail) = next_tails.get(&revision.track_id) {
                self.tails.insert(revision.track_id, *tail);
            }
            ids.push(revision.id);
        }
        Ok(ids)
    }

    pub fn history(&self, photo_id: u64) -> Result<ProjectHistory> {
        let track_id = self.track_for_photo(photo_id)?;
        let track = self
            .store
            .store()
            .track(track_id)?
            .context("photo revision track is missing")?;
        Ok(ProjectHistory {
            photo_id,
            track_id,
            root: track.root_revision,
            current: self.photo_cursor(photo_id)?,
            revisions: self.store.store().revisions_for_track(track_id)?,
            sessions: self.store.store().sessions_on_track(track_id)?,
        })
    }

    pub fn move_to(&mut self, photo_id: u64, target: RevisionId) -> Result<Project> {
        let track = self.track_for_photo(photo_id)?;
        let revision = self
            .store
            .store()
            .revision(target)?
            .context("selected Lumen revision is missing")?;
        if revision.track_id != track {
            bail!("revision {target} belongs to a different photo");
        }
        let target = self
            .store
            .store()
            .newest_compatible_ancestor(target, &PhotoCompatibility)?;
        let current = self.photo_cursor(photo_id)?;
        if target != current {
            self.store
                .mutate(|store| store.move_session(self.session_id, current, target))?;
            self.photo_cursors.insert(photo_id, target);
        }
        self.load_session_project(Some(photo_id))
    }

    pub fn sync_together(&mut self) -> Result<(CollaborationSync, Option<Project>)> {
        let sync = self
            .store
            .mutate(|store| store.sync_together(self.session_id))?;
        if let CollaborationSync::Advanced {
            collaboration, to, ..
        } = &sync
        {
            if collaboration.track_id == self.info.default_track_id {
                self.catalog_cursor = *to;
            } else if let Some(photo_id) = self.photo_id_for_track(collaboration.track_id) {
                self.photo_cursors.insert(photo_id, *to);
            }
            return Ok((sync, Some(self.load_session_project(None)?)));
        }
        Ok((sync, None))
    }

    pub fn undo(&mut self, photo_id: u64) -> Result<Project> {
        let current = self.photo_cursor(photo_id)?;
        let revision = self
            .store
            .store()
            .revision(current)?
            .context("current photo revision is missing")?;
        let parent = revision.parent_id.context("nothing to undo")?;
        self.store
            .mutate(|store| store.remember_child(self.session_id, parent, current))?;
        self.move_to(photo_id, parent)
    }

    pub fn redo(&mut self, photo_id: u64) -> Result<Project> {
        let current = self.photo_cursor(photo_id)?;
        let target = match self
            .store
            .store()
            .preferred_child(self.session_id, current)?
        {
            Some(preferred) => preferred,
            None => match self.store.store().children(current)?.as_slice() {
                [only] => only.id,
                [] => bail!("nothing to redo"),
                _ => bail!("choose which future to follow"),
            },
        };
        self.move_to(photo_id, target)
    }

    pub fn can_undo(&self, photo_id: u64) -> bool {
        self.photo_cursor(photo_id)
            .ok()
            .and_then(|cursor| self.store.store().revision(cursor).ok().flatten())
            .is_some_and(|revision| revision.parent_id.is_some())
    }

    pub fn can_redo(&self, photo_id: u64) -> bool {
        let Ok(cursor) = self.photo_cursor(photo_id) else {
            return false;
        };
        self.store
            .store()
            .preferred_child(self.session_id, cursor)
            .ok()
            .flatten()
            .is_some()
            || self
                .store
                .store()
                .children(cursor)
                .is_ok_and(|children| !children.is_empty())
    }

    pub fn cursor(&self, photo_id: u64) -> Result<RevisionId> {
        self.photo_cursor(photo_id)
    }

    pub fn session_id(&self) -> SessionId {
        self.session_id
    }

    pub fn actor(&self) -> &Actor {
        &self.actor
    }

    pub fn pending_publish_error(&self) -> Option<String> {
        self.store.pending_publish_error()
    }

    pub fn last_publish_stats(&self) -> PublishStats {
        self.store.last_publish_stats()
    }

    pub fn checkpoint(&self) -> Result<()> {
        Ok(self.store.publish()?)
    }

    fn load_session_project(&mut self, selected: Option<u64>) -> Result<Project> {
        let (catalog, catalog_tail) = load_catalog(&self.store, self.catalog_cursor)?;
        self.tails.insert(self.info.default_track_id, catalog_tail);
        self.photo_tracks = photo_track_map(&self.store)?;
        let mut photos = Vec::with_capacity(catalog.photo_ids().len());
        for id in catalog.photo_ids() {
            let track = self.track_for_photo(*id)?;
            let session = self
                .store
                .store()
                .session_on_track(self.session_id, track)?
                .with_context(|| format!("session has no cursor for photo {id}"))?;
            let cursor = self
                .store
                .store()
                .newest_compatible_ancestor(session.cursor, &PhotoCompatibility)?;
            if cursor != session.cursor {
                self.store
                    .mutate(|store| store.move_session(self.session_id, session.cursor, cursor))?;
            }
            let (photo, reference, tail) = load_photo(&self.store, &self.info, cursor)?;
            self.photo_cursors.insert(*id, cursor);
            self.photo_assets.insert(*id, reference);
            self.tails.insert(track, tail);
            photos.push(photo);
        }
        Ok(catalog.assemble(photos, selected))
    }

    fn track_for_photo(&self, photo_id: u64) -> Result<TrackId> {
        self.photo_tracks
            .get(&photo_id)
            .copied()
            .with_context(|| format!("photo {photo_id} does not have a revision track"))
    }

    fn photo_id_for_track(&self, track: TrackId) -> Option<u64> {
        self.photo_tracks
            .iter()
            .find_map(|(photo_id, candidate)| (*candidate == track).then_some(*photo_id))
    }

    fn photo_cursor(&self, photo_id: u64) -> Result<RevisionId> {
        self.photo_cursors
            .get(&photo_id)
            .copied()
            .with_context(|| format!("photo {photo_id} does not have a session cursor"))
    }

    fn require_allowed_track(&self, track: TrackId) -> Result<()> {
        if self
            .restricted_track
            .is_some_and(|allowed| allowed != track)
        {
            bail!("this agent session is scoped to a different photo");
        }
        Ok(())
    }
}

fn load_catalog(
    store: &LiveRevisionStore,
    cursor: RevisionId,
) -> Result<(CatalogState, SnapshotTail)> {
    let plan = store.store().replay_plan(cursor, &CatalogCompatibility)?;
    let mut state = decode_catalog_snapshot(&plan.snapshot)?;
    let tail = snapshot_tail(&plan.steps);
    for step in plan.steps {
        state = decode_catalog_operation(&step.operations)?;
    }
    Ok((state, tail))
}

fn load_photo(
    store: &LiveRevisionStore,
    info: &ProjectInfo,
    cursor: RevisionId,
) -> Result<(Photo, AssetReference, SnapshotTail)> {
    let plan = store.store().replay_plan(cursor, &PhotoCompatibility)?;
    let mut photo = decode_photo_snapshot(&plan.snapshot)?;
    let tail = snapshot_tail(&plan.steps);
    for step in plan.steps {
        photo = decode_photo_operation(&step.operations)?;
    }
    let reference = AssetReference::parse(&photo.path)
        .context("photo revision does not reference an embedded original")?;
    photo.path = materialize(store, info, &reference)?;
    Ok((photo, reference, tail))
}

fn snapshot_tail(steps: &[spectrum_revisions::ReplayStep]) -> SnapshotTail {
    SnapshotTail {
        commands: steps
            .iter()
            .map(|step| u64::from(step.revision.command_count))
            .sum(),
        operation_bytes: steps.iter().map(|step| step.operations.bytes.len()).sum(),
    }
}

fn prepare_new_photos(photos: &[Photo]) -> Result<Vec<(Photo, PreparedPhoto)>> {
    photos
        .iter()
        .map(|photo| Ok((photo.clone(), PreparedPhoto::new(photo)?)))
        .collect()
}

fn new_photo_track(photo: &Photo, prepared: &PreparedPhoto) -> Result<NewTrack> {
    Ok(NewTrack {
        kind: PHOTO_TRACK_KIND.into(),
        label: photo_track_label(photo.id),
        root_label: Some(format!("Imported {}", photo.name)),
        initial_snapshots: vec![photo_snapshot(photo, &prepared.reference)?],
        assets: vec![prepared.asset.clone()],
    })
}

fn photo_track_map(store: &LiveRevisionStore) -> Result<HashMap<u64, TrackId>> {
    Ok(store
        .store()
        .tracks()?
        .into_iter()
        .filter(|track| track.kind == PHOTO_TRACK_KIND)
        .filter_map(|track| photo_id_from_track_label(&track.label).map(|id| (id, track.id)))
        .collect())
}

fn photo_track(store: &LiveRevisionStore, photo_id: u64) -> Result<TrackId> {
    photo_track_map(store)?
        .get(&photo_id)
        .copied()
        .with_context(|| format!("photo {photo_id} does not have a revision track"))
}

fn source_session_from(sessions: &[Session], requested: Option<SessionId>) -> Result<SessionId> {
    match requested {
        Some(source) => {
            let session = sessions
                .iter()
                .find(|session| session.id == source)
                .with_context(|| format!("source session {source} does not exist"))?;
            if session.actor.kind != ActorKind::Human {
                bail!("source session {source} does not belong to a human");
            }
            Ok(source)
        }
        None => sessions
            .iter()
            .filter(|session| session.actor.kind == ActorKind::Human)
            .max_by_key(|session| session.updated_at_ms)
            .map(|session| session.id)
            .context("this project does not have a human session to collaborate from"),
    }
}

fn upgrade_legacy_if_needed(
    store: &mut LiveRevisionStore,
    info: &ProjectInfo,
    session_id: SessionId,
) -> Result<()> {
    let latest = store
        .store()
        .most_recent_cursor_for_track(info.default_track_id)?;
    if store
        .store()
        .replay_plan(latest, &CatalogCompatibility)
        .is_ok()
    {
        return Ok(());
    }
    let project = load_legacy_project(store, info, latest)?;
    let catalog = CatalogState::from_project(&project);
    let prepared = prepare_new_photos(&project.photos)?;
    let tracks = prepared
        .iter()
        .map(|(photo, prepared)| new_photo_track(photo, prepared))
        .collect::<Result<Vec<_>>>()?;
    let catalog_payload = catalog_snapshot(&catalog)?;
    let migration_session = if store.store().session(session_id)?.is_some() {
        session_id
    } else {
        store
            .store()
            .sessions()?
            .first()
            .map(|session| session.id)
            .context("legacy Lumen project has no session for migration")?
    };
    store.mutate(|inner| {
        inner.add_snapshot(latest, catalog_payload)?;
        if !tracks.is_empty() {
            inner.create_tracks(migration_session, env!("CARGO_PKG_VERSION"), tracks)?;
        }
        Ok(())
    })?;
    Ok(())
}

fn load_legacy_project(
    store: &LiveRevisionStore,
    info: &ProjectInfo,
    cursor: RevisionId,
) -> Result<Project> {
    let plan = store.store().replay_plan(cursor, &LegacyCompatibility)?;
    let mut project = decode_legacy_snapshot(&plan.snapshot)?;
    materialize_project_assets(store, info, &mut project)?;
    for step in plan.steps {
        let mut commands: Vec<Command> = serde_json::from_slice(&step.operations.bytes)
            .context("invalid legacy Lumen operation batch")?;
        for command in &mut commands {
            if let Command::Import { paths } = command {
                for path in paths {
                    if let Some(reference) = AssetReference::parse(path) {
                        *path = materialize(store, info, &reference)?;
                    }
                }
            }
        }
        for command in commands {
            apply_replay_command(&mut project, command)?;
        }
    }
    Ok(project)
}

fn materialize_project_assets(
    store: &LiveRevisionStore,
    info: &ProjectInfo,
    project: &mut Project,
) -> Result<()> {
    for photo in &mut project.photos {
        if let Some(reference) = AssetReference::parse(&photo.path) {
            photo.path = materialize(store, info, &reference)?;
        }
    }
    Ok(())
}

fn materialize(
    store: &LiveRevisionStore,
    info: &ProjectInfo,
    reference: &AssetReference,
) -> Result<PathBuf> {
    let asset = store
        .store()
        .asset_record(reference.id)?
        .with_context(|| format!("embedded Lumen asset {} is missing", reference.id))?;
    let path = reference.materialized_path(&info.project_id.to_string());
    let directory = path
        .parent()
        .context("materialized photo has no directory")?;
    fs::create_dir_all(directory)?;
    let valid = fs::read(&path)
        .ok()
        .is_some_and(|bytes| spectrum_revisions::AssetId::for_bytes(&bytes) == reference.id);
    if !valid {
        let temporary = directory.join(format!("{}.tmp", reference.id));
        fs::write(&temporary, &asset.bytes)?;
        if path.exists() {
            fs::remove_file(&path)?;
        }
        fs::rename(temporary, &path)?;
    }
    Ok(path)
}

fn checked_project_info(store: &LiveRevisionStore, path: &Path) -> Result<ProjectInfo> {
    let info = store.store().project_info()?;
    if info.application_id != APPLICATION_ID {
        bail!(
            "{} is a {} project, not a Lumen project",
            path.display(),
            info.application_id
        );
    }
    Ok(info)
}

#[cfg(not(test))]
fn live_cache_root(_project_path: &Path) -> Result<PathBuf> {
    eframe::storage_dir("Lumen")
        .map(|directory| directory.join("Revision Cache"))
        .context("Lumen could not locate its local revision cache")
}

#[cfg(test)]
fn live_cache_root(project_path: &Path) -> Result<PathBuf> {
    Ok(project_path
        .parent()
        .context("test project has no parent directory")?
        .join(".revision-cache"))
}
