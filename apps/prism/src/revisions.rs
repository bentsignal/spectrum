use std::{
    collections::HashSet,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use flate2::{Compression, read::ZlibDecoder, write::ZlibEncoder};
use spectrum_revisions::{
    Actor, ActorKind, AppendRevision, Asset, AssetId, Collaboration, CollaborationMode,
    CollaborationSync, Compatibility, Encoding, LiveRevisionStore, NewProject, Payload,
    ProjectInfo, Revision, RevisionId, Session, SessionId, TrackId,
};

use crate::{Command, Document, LayerKind, apply_command};

const APPLICATION_ID: &str = "spectrum.prism";
const SNAPSHOT_FAMILY: &str = "spectrum.prism.document";
const OPERATIONS_FAMILY: &str = "spectrum.prism.commands";
const LEGACY_SNAPSHOT_VERSION: u32 = 1;
const COMPRESSED_SNAPSHOT_VERSION: u32 = 2;
const OPERATIONS_VERSION: u32 = 1;
const DEFLATE_CAPABILITY: &str = "deflate";
const SNAPSHOT_COMMAND_BUDGET: u64 = 100;
const SNAPSHOT_OPERATION_BYTE_BUDGET: usize = 64 * 1024;
const ASSET_PREFIX: &str = "spectrum-asset:";

struct PrismCompatibility;

impl Compatibility for PrismCompatibility {
    fn supports_snapshot(&self, encoding: &Encoding) -> bool {
        encoding.family == SNAPSHOT_FAMILY
            && match encoding.version {
                LEGACY_SNAPSHOT_VERSION => encoding.required_capabilities.is_empty(),
                COMPRESSED_SNAPSHOT_VERSION => {
                    encoding.required_capabilities == [DEFLATE_CAPABILITY]
                }
                _ => false,
            }
    }

    fn supports_operations(&self, encoding: &Encoding) -> bool {
        encoding.family == OPERATIONS_FAMILY
            && encoding.version <= OPERATIONS_VERSION
            && encoding.required_capabilities.is_empty()
    }
}

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

pub struct DurableProject {
    store: LiveRevisionStore,
    info: ProjectInfo,
    document_track: TrackId,
    actor: Actor,
    session_id: SessionId,
    cursor: RevisionId,
    snapshot_tail: SnapshotTail,
}

#[derive(Clone, Debug)]
pub struct ProjectHistory {
    pub root: RevisionId,
    pub current: RevisionId,
    pub revisions: Vec<Revision>,
    pub sessions: Vec<Session>,
}

impl DurableProject {
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
        document: &Document,
        actor: Actor,
        session_id: SessionId,
    ) -> Result<(Self, Document)> {
        if path.exists() && path.metadata()?.len() > 0 {
            bail!("refusing to replace existing project {}", path.display());
        }
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent)?;
        }
        let prepared = PreparedSnapshot::legacy(document)?;
        let project_actor = actor.clone();
        let (store, info) = LiveRevisionStore::create(
            path,
            &live_cache_root(path)?,
            NewProject {
                application_id: APPLICATION_ID.into(),
                application_version: env!("CARGO_PKG_VERSION").into(),
                actor,
                session_id,
                root_label: Some("Created project".into()),
                track_kind: SNAPSHOT_FAMILY.into(),
                track_label: "Document".into(),
                initial_snapshots: vec![prepared.payload],
                assets: prepared.assets,
            },
        )?;
        let cursor = info.root_revision;
        let document_track = info.default_track_id;
        let project = Self {
            store,
            info,
            document_track,
            actor: project_actor,
            session_id,
            cursor,
            snapshot_tail: SnapshotTail::default(),
        };
        let (materialized, _) = project.load(cursor)?;
        Ok((project, materialized))
    }

    pub fn open(path: &Path, actor: Actor, session_id: SessionId) -> Result<(Self, Document)> {
        let mut store = LiveRevisionStore::open(path, &live_cache_root(path)?)?;
        let info = store.store().project_info()?;
        if info.application_id != APPLICATION_ID {
            bail!(
                "{} is a {} project, not a Prism project",
                path.display(),
                info.application_id
            );
        }
        let latest = store
            .store()
            .most_recent_cursor_for_track(info.default_track_id)?;
        let cursor = store
            .store()
            .newest_compatible_ancestor(latest, &PrismCompatibility)?;
        let session =
            store.mutate(|store| store.resume_session(session_id, actor.clone(), cursor))?;
        let cursor = store
            .store()
            .newest_compatible_ancestor(session.cursor, &PrismCompatibility)?;
        if cursor != session.cursor {
            store.mutate(|store| store.move_session(session_id, session.cursor, cursor))?;
        }
        let mut project = Self {
            store,
            document_track: info.default_track_id,
            info,
            actor,
            session_id,
            cursor,
            snapshot_tail: SnapshotTail::default(),
        };
        let (document, snapshot_tail) = project.load(cursor)?;
        project.snapshot_tail = snapshot_tail;
        Ok((project, document))
    }

    pub fn open_session(path: &Path, session_id: SessionId) -> Result<(Self, Document)> {
        let mut store = LiveRevisionStore::open(path, &live_cache_root(path)?)?;
        let info = checked_project_info(&store, path)?;
        let session = store
            .store()
            .session_on_track(session_id, info.default_track_id)?
            .with_context(|| format!("agent session {session_id} does not exist"))?;
        let cursor = store
            .store()
            .newest_compatible_ancestor(session.cursor, &PrismCompatibility)?;
        if cursor != session.cursor {
            store.mutate(|store| store.move_session(session_id, session.cursor, cursor))?;
        }
        let mut project = Self {
            store,
            document_track: info.default_track_id,
            info,
            actor: session.actor,
            session_id,
            cursor,
            snapshot_tail: SnapshotTail::default(),
        };
        let (document, snapshot_tail) = project.load(cursor)?;
        project.snapshot_tail = snapshot_tail;
        Ok((project, document))
    }

    pub fn start_collaboration(
        path: &Path,
        source_session: Option<SessionId>,
        agent: Actor,
        mode: CollaborationMode,
    ) -> Result<Collaboration> {
        let mut store = LiveRevisionStore::open(path, &live_cache_root(path)?)?;
        let info = checked_project_info(&store, path)?;
        let sessions = store.store().sessions_on_track(info.default_track_id)?;
        let source_session = match source_session {
            Some(source) => {
                let session = sessions
                    .iter()
                    .find(|session| session.id == source)
                    .with_context(|| format!("source session {source} does not exist"))?;
                if session.actor.kind != ActorKind::Human {
                    bail!("source session {source} does not belong to a human");
                }
                source
            }
            None => {
                sessions
                    .iter()
                    .find(|session| session.actor.kind == ActorKind::Human)
                    .context("this project does not have a human session to collaborate from")?
                    .id
            }
        };
        store
            .mutate(|store| {
                store.start_collaboration(source_session, info.default_track_id, agent, mode)
            })
            .map_err(Into::into)
    }

    pub fn collaboration(path: &Path, agent_session: SessionId) -> Result<Collaboration> {
        let store = LiveRevisionStore::open(path, &live_cache_root(path)?)?;
        checked_project_info(&store, path)?;
        store
            .store()
            .collaboration(agent_session)?
            .with_context(|| format!("agent session {agent_session} is not a collaboration"))
    }

    pub fn load_current(path: &Path) -> Result<Document> {
        let store = LiveRevisionStore::open(path, &live_cache_root(path)?)?;
        let info = store.store().project_info()?;
        if info.application_id != APPLICATION_ID {
            bail!(
                "{} is a {} project, not a Prism project",
                path.display(),
                info.application_id
            );
        }
        let latest = store
            .store()
            .most_recent_cursor_for_track(info.default_track_id)?;
        let cursor = store
            .store()
            .newest_compatible_ancestor(latest, &PrismCompatibility)?;
        let project = Self {
            store,
            document_track: info.default_track_id,
            info,
            actor: Actor {
                id: "local:read-only".into(),
                display_name: "Read-only Prism client".into(),
                kind: spectrum_revisions::ActorKind::Agent,
            },
            session_id: SessionId::new(),
            cursor,
            snapshot_tail: SnapshotTail::default(),
        };
        Ok(project.load(cursor)?.0)
    }

    pub fn commit(
        &mut self,
        commands: &[Command],
        document: &Document,
        label: impl Into<String>,
    ) -> Result<RevisionId> {
        if commands.is_empty() {
            bail!("cannot commit an empty Prism action");
        }
        let operations = PreparedOperations::new(commands)?;
        let next_tail = self
            .snapshot_tail
            .after(commands.len(), operations.payload.bytes.len());
        let snapshot = next_tail
            .needs_snapshot()
            .then(|| PreparedSnapshot::compressed(document))
            .transpose()?;
        let PreparedOperations {
            payload: operation_payload,
            assets: operation_assets,
        } = operations;
        let (snapshots, snapshot_assets) = snapshot.map_or_else(
            || (Vec::new(), Vec::new()),
            |snapshot| (vec![snapshot.payload], snapshot.assets),
        );
        let mut seen = HashSet::new();
        let assets = operation_assets
            .into_iter()
            .chain(snapshot_assets)
            .filter(|asset| seen.insert(asset.id))
            .collect();
        let request = AppendRevision {
            track_id: self.document_track,
            session_id: self.session_id,
            expected_parent: self.cursor,
            application_version: env!("CARGO_PKG_VERSION").into(),
            label: Some(label.into()),
            command_count: commands.len().try_into().unwrap_or(u32::MAX),
            operation_payloads: vec![operation_payload],
            snapshots,
            assets,
        };
        let revision = self.store.mutate(|store| store.append(request))?;
        self.cursor = revision.id;
        self.snapshot_tail = if next_tail.needs_snapshot() {
            SnapshotTail::default()
        } else {
            next_tail
        };
        Ok(revision.id)
    }

    pub fn move_to(&mut self, target: RevisionId) -> Result<Document> {
        let target = self
            .store
            .store()
            .newest_compatible_ancestor(target, &PrismCompatibility)?;
        if target == self.cursor {
            return Ok(self.load(target)?.0);
        }
        self.store
            .mutate(|store| store.move_session(self.session_id, self.cursor, target))?;
        self.cursor = target;
        let (document, snapshot_tail) = self.load(target)?;
        self.snapshot_tail = snapshot_tail;
        Ok(document)
    }

    pub fn history(&self) -> Result<ProjectHistory> {
        Ok(ProjectHistory {
            root: self.info.root_revision,
            current: self.cursor,
            revisions: self
                .store
                .store()
                .revisions_for_track(self.document_track)?,
            sessions: self.store.store().sessions_on_track(self.document_track)?,
        })
    }

    pub fn sync_together(&mut self) -> Result<(CollaborationSync, Option<Document>)> {
        let sync = self
            .store
            .mutate(|store| store.sync_together(self.session_id))?;
        if let CollaborationSync::Advanced { to, .. } = &sync {
            self.cursor = *to;
            let (document, snapshot_tail) = self.load(*to)?;
            self.snapshot_tail = snapshot_tail;
            return Ok((sync, Some(document)));
        }
        Ok((sync, None))
    }

    pub fn undo(&mut self) -> Result<Document> {
        let current = self
            .store
            .store()
            .revision(self.cursor)?
            .context("current Prism revision is missing")?;
        let parent = current.parent_id.context("nothing to undo")?;
        self.store
            .mutate(|store| store.remember_child(self.session_id, parent, self.cursor))?;
        self.move_to(parent)
    }

    pub fn redo(&mut self) -> Result<Document> {
        let preferred = self
            .store
            .store()
            .preferred_child(self.session_id, self.cursor)?;
        let target = match preferred {
            Some(preferred) => preferred,
            None => {
                let children = self.store.store().children(self.cursor)?;
                match children.as_slice() {
                    [only] => only.id,
                    [] => bail!("nothing to redo"),
                    _ => bail!("choose which future to follow"),
                }
            }
        };
        self.move_to(target)
    }

    pub fn can_undo(&self) -> bool {
        self.store
            .store()
            .revision(self.cursor)
            .ok()
            .flatten()
            .is_some_and(|revision| revision.parent_id.is_some())
    }

    pub fn can_redo(&self) -> bool {
        self.store
            .store()
            .preferred_child(self.session_id, self.cursor)
            .ok()
            .flatten()
            .is_some()
            || self
                .store
                .store()
                .children(self.cursor)
                .is_ok_and(|children| !children.is_empty())
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

    pub fn project_info(&self) -> &ProjectInfo {
        &self.info
    }

    pub fn pending_publish_error(&self) -> Option<String> {
        self.store.pending_publish_error()
    }

    pub fn checkpoint(&self) -> Result<()> {
        self.store.publish()?;
        Ok(())
    }

    fn load(&self, target: RevisionId) -> Result<(Document, SnapshotTail)> {
        let plan = self
            .store
            .store()
            .replay_plan(target, &PrismCompatibility)?;
        let snapshot_bytes = decode_snapshot(&plan.snapshot)?;
        let mut document: Document =
            serde_json::from_slice(&snapshot_bytes).context("invalid Prism snapshot")?;
        self.materialize_assets(&mut document)?;
        document.migrate()?;
        let snapshot_tail = SnapshotTail {
            commands: plan
                .steps
                .iter()
                .map(|step| u64::from(step.revision.command_count))
                .sum(),
            operation_bytes: plan
                .steps
                .iter()
                .map(|step| step.operations.bytes.len())
                .sum(),
        };
        for step in plan.steps {
            let mut commands: Vec<Command> = serde_json::from_slice(&step.operations.bytes)
                .context("invalid Prism operation batch")?;
            self.materialize_command_assets(&mut commands)?;
            for command in commands {
                apply_command(&mut document, command)?;
            }
        }
        Ok((document, snapshot_tail))
    }

    fn materialize_assets(&self, document: &mut Document) -> Result<()> {
        for layer in &mut document.layers {
            if let LayerKind::Raster {
                path,
                original_path,
            } = &mut layer.kind
                && let Some(reference) = AssetReference::parse(path)
            {
                *path = self.materialize(reference)?;
                *original_path = None;
            }
        }
        Ok(())
    }

    fn materialize_command_assets(&self, commands: &mut [Command]) -> Result<()> {
        for command in commands {
            if let Command::AddRaster { path, .. } = command
                && let Some(reference) = AssetReference::parse(path)
            {
                *path = self.materialize(reference)?;
            }
        }
        Ok(())
    }

    fn materialize(&self, reference: AssetReference) -> Result<PathBuf> {
        let asset = self
            .store
            .store()
            .asset_record(reference.id)?
            .with_context(|| format!("embedded Prism asset {} is missing", reference.id))?;
        let directory = std::env::temp_dir()
            .join("spectrum-prism-cache")
            .join(self.info.project_id.to_string());
        fs::create_dir_all(&directory)?;
        let path = directory.join(format!("{}.{}", reference.id, reference.extension));
        let valid = fs::read(&path)
            .ok()
            .is_some_and(|bytes| AssetId::for_bytes(&bytes) == reference.id);
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
}

fn checked_project_info(store: &LiveRevisionStore, path: &Path) -> Result<ProjectInfo> {
    let info = store.store().project_info()?;
    if info.application_id != APPLICATION_ID {
        bail!(
            "{} is a {} project, not a Prism project",
            path.display(),
            info.application_id
        );
    }
    Ok(info)
}

#[cfg(not(test))]
fn live_cache_root(_project_path: &Path) -> Result<PathBuf> {
    eframe::storage_dir("Prism")
        .map(|directory| directory.join("Revision Cache"))
        .context("Prism could not locate its local revision cache")
}

#[cfg(test)]
fn live_cache_root(project_path: &Path) -> Result<PathBuf> {
    Ok(project_path
        .parent()
        .context("test project has no parent directory")?
        .join(".revision-cache"))
}

struct PreparedSnapshot {
    payload: Payload,
    assets: Vec<Asset>,
}

impl PreparedSnapshot {
    fn legacy(document: &Document) -> Result<Self> {
        Self::prepare(document, false)
    }

    fn compressed(document: &Document) -> Result<Self> {
        Self::prepare(document, true)
    }

    fn prepare(document: &Document, compressed: bool) -> Result<Self> {
        let mut portable = document.clone();
        let mut assets = Vec::new();
        for layer in &mut portable.layers {
            if let LayerKind::Raster {
                path,
                original_path,
            } = &mut layer.kind
            {
                let prepared = prepare_asset(path)?;
                *path = prepared.reference.path();
                *original_path = None;
                assets.push(prepared.asset);
            }
        }
        let serialized = serde_json::to_vec(&portable)?;
        let payload = if compressed {
            Payload::new(
                Encoding::new(SNAPSHOT_FAMILY, COMPRESSED_SNAPSHOT_VERSION)
                    .requiring(DEFLATE_CAPABILITY),
                deflate(&serialized)?,
            )
        } else {
            Payload::new(
                Encoding::new(SNAPSHOT_FAMILY, LEGACY_SNAPSHOT_VERSION),
                serialized,
            )
        };
        Ok(Self { payload, assets })
    }
}

struct PreparedOperations {
    payload: Payload,
    assets: Vec<Asset>,
}

impl PreparedOperations {
    fn new(commands: &[Command]) -> Result<Self> {
        let mut portable = commands.to_vec();
        let mut assets = Vec::new();
        for command in &mut portable {
            if let Command::AddRaster { path, .. } = command {
                let prepared = prepare_asset(path)?;
                *path = prepared.reference.path();
                assets.push(prepared.asset);
            }
        }
        Ok(Self {
            payload: Payload::new(
                Encoding::new(OPERATIONS_FAMILY, OPERATIONS_VERSION),
                serde_json::to_vec(&portable)?,
            ),
            assets,
        })
    }
}

fn deflate(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(bytes)?;
    Ok(encoder.finish()?)
}

fn decode_snapshot(payload: &Payload) -> Result<Vec<u8>> {
    match payload.encoding.version {
        LEGACY_SNAPSHOT_VERSION if payload.encoding.required_capabilities.is_empty() => {
            Ok(payload.bytes.clone())
        }
        COMPRESSED_SNAPSHOT_VERSION
            if payload.encoding.required_capabilities == [DEFLATE_CAPABILITY] =>
        {
            let mut decoded = Vec::new();
            ZlibDecoder::new(payload.bytes.as_slice()).read_to_end(&mut decoded)?;
            Ok(decoded)
        }
        _ => bail!("unsupported Prism snapshot encoding"),
    }
}

struct PreparedAsset {
    reference: AssetReference,
    asset: Asset,
}

fn prepare_asset(path: &Path) -> Result<PreparedAsset> {
    if let Some(reference) = AssetReference::parse(path) {
        bail!(
            "cannot prepare unresolved embedded asset reference {}",
            reference.id
        );
    }
    let bytes = fs::read(path).with_context(|| format!("could not embed {}", path.display()))?;
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(sanitize_extension)
        .filter(|extension| !extension.is_empty())
        .unwrap_or_else(|| "bin".into());
    let asset = Asset::new(media_type(&extension), bytes);
    Ok(PreparedAsset {
        reference: AssetReference {
            id: asset.id,
            extension,
        },
        asset,
    })
}

struct AssetReference {
    id: AssetId,
    extension: String,
}

impl AssetReference {
    fn parse(path: &Path) -> Option<Self> {
        let value = path.to_str()?.strip_prefix(ASSET_PREFIX)?;
        let (hash, extension) = value.split_once('.')?;
        let id = AssetId::from_hex(hash)?;
        let extension = sanitize_extension(extension);
        if extension.is_empty() {
            return None;
        }
        Some(Self { id, extension })
    }

    fn path(&self) -> PathBuf {
        PathBuf::from(format!("{ASSET_PREFIX}{}.{}", self.id, self.extension))
    }
}

fn sanitize_extension(extension: &str) -> String {
    extension
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .take(12)
        .collect::<String>()
        .to_ascii_lowercase()
}

fn media_type(extension: &str) -> &'static str {
    match extension {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "tif" | "tiff" => "image/tiff",
        "webp" => "image/webp",
        _ => "application/octet-stream",
    }
}
