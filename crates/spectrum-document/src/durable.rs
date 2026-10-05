//! A document's revision file: snapshots and command batches on one track,
//! the files they use, and the sessions (people and agents) moving through
//! its history.
use std::{
    collections::HashSet,
    fs,
    io::{Read, Write},
    marker::PhantomData,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use flate2::{Compression, read::ZlibDecoder, write::ZlibEncoder};
use spectrum_revisions::{
    Actor, ActorKind, AppendRevision, Asset, Collaboration, CollaborationMode, CollaborationSync,
    Compatibility, Encoding, LiveRevisionStore, NewProject, Payload, ProjectInfo, Revision,
    RevisionId, Session, SessionId, TrackId,
};

use crate::{
    Model,
    files::{Reference, embed, stage},
};

const VERSION: u32 = 1;
const DEFLATE: &str = "deflate";
/// A snapshot is stored once this many commands, or bytes of commands,
/// follow the last one, so opening replays a bounded tail.
const SNAPSHOT_COMMANDS: u64 = 100;
const SNAPSHOT_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, Default)]
struct Tail {
    commands: u64,
    bytes: usize,
}

impl Tail {
    fn after(self, commands: usize, bytes: usize) -> Self {
        Self {
            commands: self.commands.saturating_add(commands as u64),
            bytes: self.bytes.saturating_add(bytes),
        }
    }

    fn full(self) -> bool {
        self.commands >= SNAPSHOT_COMMANDS || self.bytes >= SNAPSHOT_BYTES
    }
}

/// A document's history: every revision, which one this session is on, and
/// every session.
#[derive(Clone, Debug)]
pub struct History {
    pub root: RevisionId,
    pub current: RevisionId,
    pub revisions: Vec<Revision>,
    pub sessions: Vec<Session>,
}

struct Families<M>(PhantomData<M>);

impl<M: Model> Families<M> {
    fn snapshot() -> String {
        format!("{}.snapshot", M::APPLICATION)
    }

    fn operations() -> String {
        format!("{}.operations", M::APPLICATION)
    }
}

impl<M: Model> Compatibility for Families<M> {
    fn supports_snapshot(&self, encoding: &Encoding) -> bool {
        encoding.family == Self::snapshot()
            && encoding.version == VERSION
            && (encoding.required_capabilities.is_empty()
                || encoding.required_capabilities == [DEFLATE])
    }

    fn supports_operations(&self, encoding: &Encoding) -> bool {
        encoding.family == Self::operations()
            && encoding.version == VERSION
            && encoding.required_capabilities.is_empty()
    }
}

/// Commands readied for a durable edit: the form to apply now (files
/// staged in the cache) and the form to store (files embedded).
pub(crate) struct Prepared<M: Model> {
    pub(crate) apply: Vec<M::Command>,
    operations: Payload,
    assets: Vec<Asset>,
    force_snapshot: bool,
}

/// A document's revision file, open in one session.
pub struct Durable<M: Model> {
    store: LiveRevisionStore,
    info: ProjectInfo,
    track: TrackId,
    actor: Actor,
    session_id: SessionId,
    cursor: RevisionId,
    tail: Tail,
    files: PathBuf,
    _model: PhantomData<M>,
}

/// The working copy and staged files live beside the document, so every
/// library keeps its own caches and can be moved or removed whole.
fn cache_root(path: &Path) -> Result<PathBuf> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    Ok(parent.join(".cache"))
}

impl<M: Model> Durable<M> {
    /// Creates a revision file holding `document`.
    pub fn create(
        path: &Path,
        document: &M::Document,
        actor: Actor,
        session_id: SessionId,
    ) -> Result<(Self, M::Document)> {
        if path.exists() {
            bail!("refusing to replace {}", path.display());
        }
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(parent)?;
        }
        let (snapshot, assets) = snapshot::<M>(document)?;
        let (store, info) = LiveRevisionStore::create(
            path,
            &cache_root(path)?,
            NewProject {
                application_id: M::APPLICATION.into(),
                application_version: env!("CARGO_PKG_VERSION").into(),
                actor: actor.clone(),
                session_id,
                root_label: Some(format!("Created {}", M::NOUN)),
                track_kind: Families::<M>::snapshot(),
                track_label: M::NOUN.into(),
                initial_snapshots: vec![snapshot],
                assets,
            },
        )?;
        let durable = Self::new(path, store, info, actor, session_id)?;
        let (document, _) = durable.load(durable.cursor)?;
        Ok((durable, document))
    }

    /// Opens a revision file, resuming `session_id` (or starting it at the
    /// newest revision).
    pub fn open(path: &Path, actor: Actor, session_id: SessionId) -> Result<(Self, M::Document)> {
        let mut store = LiveRevisionStore::open(path, &cache_root(path)?)?;
        let info = checked_info::<M>(&store, path)?;
        let latest = store
            .store()
            .most_recent_cursor_for_track(info.default_track_id)?;
        let session =
            store.mutate(|store| store.resume_session(session_id, actor.clone(), latest))?;
        let mut durable = Self::new(path, store, info, actor, session_id)?;
        durable.cursor = session.cursor;
        let (document, tail) = durable.load(durable.cursor)?;
        durable.tail = tail;
        Ok((durable, document))
    }

    /// Opens an existing session, such as an agent's.
    pub fn open_session(path: &Path, session_id: SessionId) -> Result<(Self, M::Document)> {
        let store = LiveRevisionStore::open(path, &cache_root(path)?)?;
        let info = checked_info::<M>(&store, path)?;
        let session = store
            .store()
            .session_on_track(session_id, info.default_track_id)?
            .with_context(|| format!("session {session_id} does not exist"))?;
        let mut durable = Self::new(path, store, info, session.actor, session_id)?;
        durable.cursor = session.cursor;
        let (document, tail) = durable.load(durable.cursor)?;
        durable.tail = tail;
        Ok((durable, document))
    }

    /// The newest document in a revision file, without joining a session.
    pub fn read(path: &Path) -> Result<M::Document> {
        let store = LiveRevisionStore::open(path, &cache_root(path)?)?;
        let info = checked_info::<M>(&store, path)?;
        let cursor = store
            .store()
            .most_recent_cursor_for_track(info.default_track_id)?;
        let reader = Actor {
            id: "spectrum:reader".into(),
            display_name: "Spectrum".into(),
            kind: ActorKind::Agent,
        };
        let mut durable = Self::new(path, store, info, reader, SessionId::new())?;
        durable.cursor = cursor;
        Ok(durable.load(cursor)?.0)
    }

    /// Starts an agent session from a person's session.
    pub fn start_collaboration(
        path: &Path,
        source: Option<SessionId>,
        agent: Actor,
        mode: CollaborationMode,
    ) -> Result<Collaboration> {
        let mut store = LiveRevisionStore::open(path, &cache_root(path)?)?;
        let info = checked_info::<M>(&store, path)?;
        let sessions = store.store().sessions_on_track(info.default_track_id)?;
        let source = match source {
            Some(source) => {
                let session = sessions
                    .iter()
                    .find(|session| session.id == source)
                    .with_context(|| format!("session {source} does not exist"))?;
                if session.actor.kind != ActorKind::Human {
                    bail!("session {source} does not belong to a person");
                }
                source
            }
            None => {
                sessions
                    .iter()
                    .find(|session| session.actor.kind == ActorKind::Human)
                    .with_context(|| {
                        format!("this {} has no person's session to work from", M::NOUN)
                    })?
                    .id
            }
        };
        Ok(store.mutate(|store| {
            store.start_collaboration(source, info.default_track_id, agent, mode)
        })?)
    }

    /// An agent session's collaboration.
    pub fn collaboration(path: &Path, agent_session: SessionId) -> Result<Collaboration> {
        let store = LiveRevisionStore::open(path, &cache_root(path)?)?;
        checked_info::<M>(&store, path)?;
        store
            .store()
            .collaboration(agent_session)?
            .with_context(|| format!("session {agent_session} is not a collaboration"))
    }

    fn new(
        path: &Path,
        store: LiveRevisionStore,
        info: ProjectInfo,
        actor: Actor,
        session_id: SessionId,
    ) -> Result<Self> {
        let files = cache_root(path)?
            .join("files")
            .join(info.project_id.to_string());
        Ok(Self {
            store,
            track: info.default_track_id,
            cursor: info.root_revision,
            info,
            actor,
            session_id,
            tail: Tail::default(),
            files,
            _model: PhantomData,
        })
    }

    /// Readies commands: files they use are embedded for storing and staged
    /// for applying.
    pub(crate) fn prepare(&self, commands: Vec<M::Command>) -> Result<Prepared<M>> {
        let mut stored = Vec::with_capacity(commands.len());
        let mut force_snapshot = false;
        for command in &commands {
            let (command, snapshot) = M::stored(command);
            force_snapshot |= snapshot;
            stored.push(command);
        }
        let mut apply = commands;
        let mut assets = Vec::new();
        for (applied, stored) in apply.iter_mut().zip(stored.iter_mut()) {
            let mut staged = Vec::new();
            M::command_files(applied, &mut |path, role| {
                let (reference, asset) = embed(path, &role)?;
                *path = stage(&self.files, &reference, &asset.bytes)?;
                staged.push(reference);
                assets.push(asset);
                Ok(())
            })?;
            let mut references = staged.into_iter();
            M::command_files(stored, &mut |path, _| {
                *path = references
                    .next()
                    .context("a command's stored files differ from its applied ones")?
                    .path();
                Ok(())
            })?;
        }
        Ok(Prepared {
            apply,
            operations: Payload::new(
                Encoding::new(Families::<M>::operations(), VERSION),
                serde_json::to_vec(&stored)?,
            ),
            assets,
            force_snapshot,
        })
    }

    /// Stores a prepared edit that produced `document`.
    pub(crate) fn commit(
        &mut self,
        prepared: Prepared<M>,
        document: &M::Document,
        label: impl Into<String>,
    ) -> Result<RevisionId> {
        let count = prepared.apply.len();
        let next = self.tail.after(count, prepared.operations.bytes.len());
        let snapshot_now = prepared.force_snapshot || next.full();
        let (snapshots, snapshot_assets) = if snapshot_now {
            let (payload, assets) = snapshot::<M>(document)?;
            (vec![payload], assets)
        } else {
            (Vec::new(), Vec::new())
        };
        let mut seen = HashSet::new();
        let assets = prepared
            .assets
            .into_iter()
            .chain(snapshot_assets)
            .filter(|asset| seen.insert(asset.id))
            .collect();
        let revision = self.store.mutate(|store| {
            store.append(AppendRevision {
                track_id: self.track,
                session_id: self.session_id,
                expected_parent: self.cursor,
                application_version: env!("CARGO_PKG_VERSION").into(),
                label: Some(label.into()),
                command_count: count.try_into().unwrap_or(u32::MAX),
                operation_payloads: vec![prepared.operations],
                snapshots,
                assets,
            })
        })?;
        self.cursor = revision.id;
        self.tail = if snapshot_now { Tail::default() } else { next };
        Ok(revision.id)
    }

    /// Moves this session to a revision, returning the document there.
    pub fn move_to(&mut self, target: RevisionId) -> Result<M::Document> {
        self.navigate(target, None)
    }

    /// Back one revision.
    pub fn undo(&mut self) -> Result<M::Document> {
        let current = self.current_revision()?;
        let parent = current.parent_id.context("nothing to undo")?;
        self.navigate(parent, Some((parent, self.cursor)))
    }

    /// Forward one revision: the one this session last left, or the only one.
    pub fn redo(&mut self) -> Result<M::Document> {
        let store = self.store.store();
        let target = match store.preferred_child(self.session_id, self.cursor)? {
            Some(preferred) => preferred,
            None => match store.children(self.cursor)?.as_slice() {
                [only] => only.id,
                [] => bail!("nothing to redo"),
                _ => bail!("choose which future to follow"),
            },
        };
        self.navigate(target, None)
    }

    fn navigate(
        &mut self,
        target: RevisionId,
        remember: Option<(RevisionId, RevisionId)>,
    ) -> Result<M::Document> {
        let (document, tail) = self.load(target)?;
        if target != self.cursor {
            if let Some((parent, child)) = remember {
                self.store
                    .mutate(|store| store.remember_child(self.session_id, parent, child))?;
            }
            let from = self.cursor;
            self.store
                .mutate(|store| store.move_session(self.session_id, from, target))?;
            self.cursor = target;
        }
        self.tail = tail;
        Ok(document)
    }

    pub fn can_undo(&self) -> bool {
        self.current_revision()
            .is_ok_and(|revision| revision.parent_id.is_some())
    }

    pub fn can_redo(&self) -> bool {
        let store = self.store.store();
        store
            .preferred_child(self.session_id, self.cursor)
            .ok()
            .flatten()
            .is_some()
            || store
                .children(self.cursor)
                .is_ok_and(|children| !children.is_empty())
    }

    pub fn history(&self) -> Result<History> {
        Ok(History {
            root: self.info.root_revision,
            current: self.cursor,
            revisions: self.store.store().revisions_for_track(self.track)?,
            sessions: self.store.store().sessions_on_track(self.track)?,
        })
    }

    /// For an agent working together with a person: follows the person's
    /// newer revisions, returning the document if it moved.
    pub fn sync_together(&mut self) -> Result<(CollaborationSync, Option<M::Document>)> {
        let sync = self
            .store
            .mutate(|store| store.sync_together(self.session_id))?;
        if let CollaborationSync::Advanced { to, .. } = &sync {
            self.cursor = *to;
            let (document, tail) = self.load(*to)?;
            self.tail = tail;
            return Ok((sync, Some(document)));
        }
        Ok((sync, None))
    }

    pub fn cursor(&self) -> RevisionId {
        self.cursor
    }

    pub fn session_id(&self) -> SessionId {
        self.session_id
    }

    pub fn actor(&self) -> &Actor {
        &self.actor
    }

    pub fn info(&self) -> &ProjectInfo {
        &self.info
    }

    pub fn current_revision(&self) -> Result<Revision> {
        self.store
            .store()
            .revision(self.cursor)?
            .with_context(|| format!("this {}'s current revision is missing", M::NOUN))
    }

    /// Why the last change could not be written to the document's file, if
    /// it could not; the change is safe in the working copy meanwhile.
    pub fn pending_publish_error(&self) -> Option<String> {
        self.store.pending_publish_error()
    }

    /// Writes the document's file now.
    pub fn checkpoint(&self) -> Result<()> {
        Ok(self.store.publish()?)
    }

    /// The document at a revision: its nearest snapshot plus the commands
    /// after it, with embedded files staged.
    fn load(&self, target: RevisionId) -> Result<(M::Document, Tail)> {
        let plan = self
            .store
            .store()
            .replay_plan(target, &Families::<M>(PhantomData))?;
        let bytes = if plan
            .snapshot
            .encoding
            .required_capabilities
            .iter()
            .any(|capability| capability == DEFLATE)
        {
            let mut inflated = Vec::new();
            ZlibDecoder::new(plan.snapshot.bytes.as_slice()).read_to_end(&mut inflated)?;
            inflated
        } else {
            plan.snapshot.bytes
        };
        let mut document: M::Document = serde_json::from_slice(&bytes)
            .with_context(|| format!("this {}'s snapshot is unreadable", M::NOUN))?;
        M::document_files(&mut document, &mut |path, _| self.restore(path))?;
        M::loaded(&mut document)?;
        let mut tail = Tail::default();
        for step in plan.steps {
            tail = tail.after(
                step.revision.command_count as usize,
                step.operations.bytes.len(),
            );
            let commands: Vec<M::Command> = serde_json::from_slice(&step.operations.bytes)
                .with_context(|| format!("a {} edit is unreadable", M::NOUN))?;
            for mut command in commands {
                M::command_files(&mut command, &mut |path, _| self.restore(path))?;
                M::apply(&mut document, command)?;
            }
        }
        Ok((document, tail))
    }

    /// Replaces a stored reference with the embedded file, staged.
    fn restore(&self, path: &mut PathBuf) -> Result<()> {
        let Some(reference) = Reference::parse(path) else {
            return Ok(());
        };
        let asset = self
            .store
            .store()
            .asset_record(reference.id)?
            .with_context(|| format!("embedded file {} is missing", reference.id))?;
        *path = stage(&self.files, &reference, &asset.bytes)?;
        Ok(())
    }
}

/// A compressed snapshot of a document, its files embedded.
fn snapshot<M: Model>(document: &M::Document) -> Result<(Payload, Vec<Asset>)> {
    let mut stored = document.clone();
    let mut assets: Vec<Asset> = Vec::new();
    M::document_files(&mut stored, &mut |path, role| {
        let (reference, asset) = embed(path, &role)?;
        *path = reference.path();
        if !assets.iter().any(|existing| existing.id == asset.id) {
            assets.push(asset);
        }
        Ok(())
    })?;
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&serde_json::to_vec(&stored)?)?;
    Ok((
        Payload::new(
            Encoding::new(Families::<M>::snapshot(), VERSION).requiring(DEFLATE),
            encoder.finish()?,
        ),
        assets,
    ))
}

fn checked_info<M: Model>(store: &LiveRevisionStore, path: &Path) -> Result<ProjectInfo> {
    let info = store.store().project_info()?;
    if info.application_id != M::APPLICATION {
        bail!(
            "{} holds a {}, not a {}",
            path.display(),
            info.application_id,
            M::NOUN
        );
    }
    Ok(info)
}
