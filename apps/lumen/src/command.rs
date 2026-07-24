use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use spectrum_revisions::{
    Actor, ActorKind, Collaboration, CollaborationMode, CollaborationSync, SessionId,
};

use crate::{
    AdjustmentPatch, Adjustments, DurableCatalog, PickState, Project, ProjectHistory,
    engine::{ExportFormat, RenderOptions, batch_destination, export_photo},
};

/// Complete application command surface shared by the GUI, CLI, and agents.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "kebab-case")]
pub enum Command {
    New {
        name: String,
    },
    Open {
        path: PathBuf,
    },
    Save {
        path: Option<PathBuf>,
    },
    Import {
        paths: Vec<PathBuf>,
    },
    Select {
        id: u64,
    },
    SetPick {
        ids: Vec<u64>,
        state: PickState,
    },
    RenameBatch {
        id: u64,
        name: String,
    },
    Adjust {
        id: u64,
        patch: AdjustmentPatch,
    },
    SetAdjustments {
        id: u64,
        adjustments: Adjustments,
    },
    Reset {
        ids: Vec<u64>,
    },
    HistoryBack {
        id: u64,
    },
    HistoryForward {
        id: u64,
    },
    HistoryJump {
        id: u64,
        index: usize,
    },
    SavePreset {
        name: String,
        from_id: u64,
    },
    ApplyPreset {
        preset_id: u64,
        ids: Vec<u64>,
    },
    DeletePreset {
        id: u64,
    },
    CopyEdits {
        id: u64,
    },
    PasteEdits {
        ids: Vec<u64>,
    },
    Remove {
        ids: Vec<u64>,
    },
    Rotate {
        id: u64,
        clockwise: bool,
    },
    FlipHorizontal {
        id: u64,
    },
    FlipVertical {
        id: u64,
    },
    Export {
        id: u64,
        path: PathBuf,
        max_size: Option<u32>,
        quality: u8,
    },
    ExportBatch {
        ids: Vec<u64>,
        directory: PathBuf,
        format: ExportFormat,
        max_size: Option<u32>,
        quality: u8,
    },
    Undo,
    Redo,
}

#[derive(Clone, Debug, Serialize)]
pub struct CommandOutput {
    pub ok: bool,
    pub action: String,
    pub message: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub photo_ids: Vec<u64>,
}

impl CommandOutput {
    fn success(action: &str, message: impl Into<String>, photo_ids: Vec<u64>) -> Self {
        Self {
            ok: true,
            action: action.to_owned(),
            message: message.into(),
            photo_ids,
        }
    }
}

pub struct Workspace {
    pub project: Project,
    pub catalog_path: Option<PathBuf>,
    pub clipboard: Option<Adjustments>,
    durable: Option<DurableCatalog>,
    legacy_photo_history: bool,
    undo: Vec<Project>,
    redo: Vec<Project>,
}

impl Default for Workspace {
    fn default() -> Self {
        Self::new(Project::default(), None)
    }
}

impl Workspace {
    pub fn new(project: Project, catalog_path: Option<PathBuf>) -> Self {
        Self {
            project,
            catalog_path,
            clipboard: None,
            durable: None,
            legacy_photo_history: true,
            undo: Vec::new(),
            redo: Vec::new(),
        }
    }
    pub fn open_as(path: &Path, actor: Actor, session_id: SessionId) -> Result<Self> {
        if DurableCatalog::looks_durable(path)? {
            let (durable, project) = DurableCatalog::open(path, actor, session_id)?;
            return Ok(Self::from_durable(path, durable, project));
        }
        let project = Project::load(path)?;
        let destination = path.with_extension("lumen");
        if destination == path {
            bail!("legacy JSON catalogs must use .lumencatalog before migration");
        }
        if destination.exists() {
            bail!(
                "could not migrate {} because {} already exists",
                path.display(),
                destination.display()
            );
        }
        let (durable, project) = DurableCatalog::create(&destination, &project, actor, session_id)?;
        Ok(Self::from_durable(&destination, durable, project))
    }

    pub fn open_session(path: &Path, session_id: SessionId) -> Result<Self> {
        if !DurableCatalog::looks_durable(path)? {
            bail!("agent sessions require a durable Lumen project");
        }
        let (durable, project) = DurableCatalog::open_session(path, session_id)?;
        Ok(Self::from_durable(path, durable, project))
    }

    pub fn create_durable(
        project: Project,
        path: &Path,
        actor: Actor,
        session_id: SessionId,
    ) -> Result<Self> {
        let (durable, project) = DurableCatalog::create(path, &project, actor, session_id)?;
        Ok(Self::from_durable(path, durable, project))
    }

    pub fn start_collaboration(
        path: &Path,
        source_session: Option<SessionId>,
        photo_id: u64,
        agent: Actor,
        mode: CollaborationMode,
    ) -> Result<Collaboration> {
        DurableCatalog::start_collaboration(path, source_session, photo_id, agent, mode)
    }

    pub fn collaboration(path: &Path, agent_session: SessionId) -> Result<Collaboration> {
        DurableCatalog::collaboration(path, agent_session)
    }

    pub fn is_durable(&self) -> bool {
        self.durable.is_some()
    }

    pub fn session_id(&self) -> Option<SessionId> {
        self.durable.as_ref().map(DurableCatalog::session_id)
    }

    pub fn history(&self) -> Result<Option<ProjectHistory>> {
        let Some(durable) = &self.durable else {
            return Ok(None);
        };
        let id = self
            .project
            .selected
            .context("select a photo to view its history")?;
        Ok(Some(durable.history(id)?))
    }

    pub fn history_for(&self, photo_id: u64) -> Result<Option<ProjectHistory>> {
        self.durable
            .as_ref()
            .map(|durable| durable.history(photo_id))
            .transpose()
    }

    pub fn move_to_revision(&mut self, target: spectrum_revisions::RevisionId) -> Result<bool> {
        let durable = self
            .durable
            .as_mut()
            .context("legacy Lumen catalogs do not have a revision tree")?;
        let photo_id = self
            .project
            .selected
            .context("select a photo before navigating history")?;
        if durable.cursor(photo_id)? == target {
            return Ok(false);
        }
        self.project = durable.move_to(photo_id, target)?;
        self.clipboard = None;
        Ok(true)
    }

    pub fn move_photo_to_revision(
        &mut self,
        photo_id: u64,
        target: spectrum_revisions::RevisionId,
    ) -> Result<bool> {
        self.project.photo(photo_id)?;
        self.project.selected = Some(photo_id);
        self.move_to_revision(target)
    }

    pub fn sync_together(&mut self) -> Result<CollaborationSync> {
        let Some(durable) = &mut self.durable else {
            return Ok(CollaborationSync::Idle);
        };
        let (sync, project) = durable.sync_together()?;
        if let Some(project) = project {
            self.project = project;
            self.clipboard = None;
        }
        Ok(sync)
    }

    pub fn checkpoint(&self) -> Result<()> {
        if let Some(durable) = &self.durable {
            durable.checkpoint()?;
        }
        Ok(())
    }

    pub fn move_project(&mut self, destination: &Path) -> Result<PathBuf> {
        let source = self
            .catalog_path
            .clone()
            .context("this Lumen project does not have a durable location")?;
        if source == destination {
            return Ok(source);
        }
        if destination.exists() {
            bail!(
                "refusing to replace existing project {}",
                destination.display()
            );
        }
        if destination.extension().and_then(|value| value.to_str()) != Some("lumen") {
            bail!("Lumen projects must use the .lumen extension");
        }
        if let Some(parent) = destination
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent)?;
        }
        let durable = self
            .durable
            .take()
            .context("legacy catalogs must be migrated before moving")?;
        durable.checkpoint()?;
        let actor = durable.actor().clone();
        let session_id = durable.session_id();
        drop(durable);
        let copied = match std::fs::rename(&source, destination) {
            Ok(()) => false,
            Err(error) if error.kind() == std::io::ErrorKind::CrossesDevices => {
                std::fs::copy(&source, destination)?;
                std::fs::File::open(destination)?.sync_all()?;
                true
            }
            Err(error) => {
                let (durable, project) = DurableCatalog::open(&source, actor, session_id)?;
                self.durable = Some(durable);
                self.project = project;
                return Err(error.into());
            }
        };
        match DurableCatalog::open(destination, actor.clone(), session_id) {
            Ok((durable, project)) => {
                self.durable = Some(durable);
                self.project = project;
                self.catalog_path = Some(destination.to_owned());
            }
            Err(error) => {
                if copied {
                    let _ = std::fs::remove_file(destination);
                } else {
                    let _ = std::fs::rename(destination, &source);
                }
                let (durable, project) = DurableCatalog::open(&source, actor, session_id)?;
                self.durable = Some(durable);
                self.project = project;
                return Err(error).context("moved Lumen project could not be reopened");
            }
        }
        if copied {
            std::fs::remove_file(&source)?;
        }
        Ok(destination.to_owned())
    }

    pub fn pending_publish_error(&self) -> Option<String> {
        self.durable
            .as_ref()
            .and_then(DurableCatalog::pending_publish_error)
    }

    pub fn last_publish_stats(&self) -> Option<spectrum_revisions::PublishStats> {
        self.durable
            .as_ref()
            .map(DurableCatalog::last_publish_stats)
    }

    pub fn can_undo(&self) -> bool {
        self.durable.as_ref().map_or_else(
            || !self.undo.is_empty(),
            |durable| self.project.selected.is_some_and(|id| durable.can_undo(id)),
        )
    }
    pub fn can_redo(&self) -> bool {
        self.durable.as_ref().map_or_else(
            || !self.redo.is_empty(),
            |durable| self.project.selected.is_some_and(|id| durable.can_redo(id)),
        )
    }

    pub fn execute(&mut self, command: Command) -> Result<CommandOutput> {
        match &command {
            Command::Undo => return self.undo(),
            Command::Redo => return self.redo(),
            Command::HistoryBack { id } if self.durable.is_some() => {
                let id = *id;
                self.project = self.durable.as_mut().unwrap().undo(id)?;
                self.project.selected = Some(id);
                self.clipboard = None;
                return Ok(CommandOutput::success(
                    "history-back",
                    format!("moved photo {id} back one edit"),
                    vec![id],
                ));
            }
            Command::HistoryForward { id } if self.durable.is_some() => {
                let id = *id;
                self.project = self.durable.as_mut().unwrap().redo(id)?;
                self.project.selected = Some(id);
                self.clipboard = None;
                return Ok(CommandOutput::success(
                    "history-forward",
                    format!("moved photo {id} forward one edit"),
                    vec![id],
                ));
            }
            Command::HistoryJump { id, index } if self.durable.is_some() => {
                let id = *id;
                let target = self
                    .durable
                    .as_ref()
                    .unwrap()
                    .history(id)?
                    .revisions
                    .get(*index)
                    .with_context(|| format!("history entry {index} does not exist"))?
                    .id;
                self.project = self.durable.as_mut().unwrap().move_to(id, target)?;
                self.project.selected = Some(id);
                self.clipboard = None;
                return Ok(CommandOutput::success(
                    "history-jump",
                    format!("moved photo {id} to history entry {index}"),
                    vec![id],
                ));
            }
            Command::Open { path } => {
                let actor = Actor {
                    id: "local:human".into(),
                    display_name: "Local User".into(),
                    kind: ActorKind::Human,
                };
                *self = Self::open_as(path, actor, SessionId::new())?;
                return Ok(CommandOutput::success("open", "opened project", vec![]));
            }
            Command::Save { path } if self.durable.is_some() => {
                if path
                    .as_ref()
                    .is_some_and(|path| Some(path.as_path()) != self.catalog_path.as_deref())
                {
                    bail!("use Move Project to relocate a durable Lumen project");
                }
                self.checkpoint()?;
                return Ok(CommandOutput::success(
                    "save",
                    "project is already current",
                    vec![],
                ));
            }
            Command::New { .. } if self.durable.is_some() => {
                bail!("create a new managed Lumen project instead of replacing this project");
            }
            _ => {}
        }
        if matches!(
            command,
            Command::Select { .. }
                | Command::CopyEdits { .. }
                | Command::Export { .. }
                | Command::ExportBatch { .. }
        ) {
            return self.execute_in_memory(command);
        }
        let replay_commands = self.replay_commands(&command)?;
        let before = self.project.clone();
        let clipboard = self.clipboard.clone();
        let undo_len = self.undo.len();
        let redo = self.redo.clone();
        let output = match self.execute_in_memory(command) {
            Ok(output) => output,
            Err(error) => {
                self.project = before;
                self.clipboard = clipboard;
                return Err(error);
            }
        };
        if self.project == before || self.durable.is_none() {
            return Ok(output);
        }
        let label = output.message.clone();
        if let Some(durable) = &mut self.durable
            && let Err(error) = durable.commit(&replay_commands, &before, &self.project, label)
        {
            self.project = before;
            self.clipboard = clipboard;
            self.undo.truncate(undo_len);
            self.redo = redo;
            return Err(error);
        }
        self.undo.clear();
        self.redo.clear();
        Ok(output)
    }

    pub fn execute_batch(&mut self, commands: Vec<Command>) -> Result<Vec<CommandOutput>> {
        if commands.is_empty() {
            bail!("command batch is empty");
        }
        if commands.iter().any(|command| {
            matches!(
                command,
                Command::New { .. }
                    | Command::Open { .. }
                    | Command::Save { .. }
                    | Command::Select { .. }
                    | Command::CopyEdits { .. }
                    | Command::Export { .. }
                    | Command::ExportBatch { .. }
                    | Command::HistoryBack { .. }
                    | Command::HistoryForward { .. }
                    | Command::HistoryJump { .. }
                    | Command::Undo
                    | Command::Redo
            )
        }) {
            bail!(
                "lifecycle, selection, clipboard, export, and history navigation commands cannot be batched"
            );
        }
        let before = self.project.clone();
        let clipboard = self.clipboard.clone();
        let mut replay = Vec::new();
        let mut outputs = Vec::with_capacity(commands.len());
        for command in commands {
            replay.extend(self.replay_commands(&command)?);
            match self.execute_in_memory(command) {
                Ok(output) => outputs.push(output),
                Err(error) => {
                    self.project = before;
                    self.clipboard = clipboard;
                    self.undo.clear();
                    self.redo.clear();
                    return Err(error);
                }
            }
        }
        if self.project != before
            && let Some(durable) = &mut self.durable
        {
            let label = if outputs.len() == 1 {
                outputs[0].message.clone()
            } else {
                format!("Applied {} actions", outputs.len())
            };
            if let Err(error) = durable.commit(&replay, &before, &self.project, label) {
                self.project = before;
                self.clipboard = clipboard;
                self.undo.clear();
                self.redo.clear();
                return Err(error);
            }
        }
        self.undo.clear();
        self.redo.clear();
        Ok(outputs)
    }

    fn execute_in_memory(&mut self, command: Command) -> Result<CommandOutput> {
        match command {
            Command::New { name } => {
                self.record_undo();
                self.project = Project::new(name);
                self.catalog_path = None;
                Ok(CommandOutput::success(
                    "new",
                    "created a new catalog",
                    vec![],
                ))
            }
            Command::Open { path } => {
                let project = Project::load(&path)?;
                self.record_undo();
                self.project = project;
                self.catalog_path = Some(path);
                Ok(CommandOutput::success("open", "opened catalog", vec![]))
            }
            Command::Save { path } => {
                let destination = path.or_else(|| self.catalog_path.clone()).ok_or_else(|| {
                    anyhow::anyhow!("a destination path is required for an unsaved catalog")
                })?;
                self.project.save(&destination)?;
                self.catalog_path = Some(destination.clone());
                Ok(CommandOutput::success(
                    "save",
                    format!("saved {}", destination.display()),
                    vec![],
                ))
            }
            Command::Import { paths } => {
                let mut updated = self.project.clone();
                let ids = updated.import(&paths)?;
                self.record_undo();
                self.project = updated;
                Ok(CommandOutput::success(
                    "import",
                    format!("imported {} photo(s)", ids.len()),
                    ids,
                ))
            }
            Command::Select { id } => {
                self.project.photo(id)?;
                self.project.selected = Some(id);
                Ok(CommandOutput::success(
                    "select",
                    format!("selected photo {id}"),
                    vec![id],
                ))
            }
            Command::SetPick { ids, state } => {
                self.ensure_ids(&ids)?;
                self.record_undo();
                for id in &ids {
                    self.project.photo_mut(*id)?.pick = state;
                }
                Ok(CommandOutput::success(
                    "set-pick",
                    format!("marked {} photo(s) as {:?}", ids.len(), state),
                    ids,
                ))
            }
            Command::RenameBatch { id, name } => {
                self.project.batch(id)?;
                self.record_undo();
                self.project.rename_batch(id, name)?;
                Ok(CommandOutput::success(
                    "rename-batch",
                    format!("renamed batch {id}"),
                    vec![],
                ))
            }
            Command::Adjust { id, patch } => {
                let mut next = self.project.photo(id)?.adjustments.clone();
                patch.apply_to(&mut next);
                self.record_undo();
                self.commit_photo_adjustments(id, "Develop adjustments", next)?;
                Ok(CommandOutput::success(
                    "adjust",
                    format!("adjusted photo {id}"),
                    vec![id],
                ))
            }
            Command::SetAdjustments { id, adjustments } => {
                self.project.photo(id)?;
                self.record_undo();
                self.commit_photo_adjustments(id, "Develop adjustments", adjustments)?;
                Ok(CommandOutput::success(
                    "adjust",
                    format!("adjusted photo {id}"),
                    vec![id],
                ))
            }
            Command::Reset { ids } => {
                self.ensure_ids(&ids)?;
                self.record_undo();
                for id in &ids {
                    self.commit_photo_adjustments(*id, "Reset all edits", Adjustments::default())?;
                }
                Ok(CommandOutput::success(
                    "reset",
                    "reset edits (undoable in history)",
                    ids,
                ))
            }
            Command::HistoryBack { id } => {
                if !self.project.photo_mut(id)?.history_back() {
                    bail!("already at the first history entry");
                }
                Ok(CommandOutput::success(
                    "history-back",
                    format!("moved photo {id} back one edit"),
                    vec![id],
                ))
            }
            Command::HistoryForward { id } => {
                if !self.project.photo_mut(id)?.history_forward() {
                    bail!("already at the latest history entry");
                }
                Ok(CommandOutput::success(
                    "history-forward",
                    format!("moved photo {id} forward one edit"),
                    vec![id],
                ))
            }
            Command::HistoryJump { id, index } => {
                self.project.photo_mut(id)?.history_jump(index)?;
                Ok(CommandOutput::success(
                    "history-jump",
                    format!("moved photo {id} to history entry {index}"),
                    vec![id],
                ))
            }
            Command::SavePreset { name, from_id } => {
                let adjustments = self.project.photo(from_id)?.adjustments.clone();
                self.record_undo();
                let id = self.project.save_preset(name, &adjustments)?;
                Ok(CommandOutput::success(
                    "save-preset",
                    format!("saved preset {id}"),
                    vec![from_id],
                ))
            }
            Command::ApplyPreset { preset_id, ids } => {
                self.ensure_ids(&ids)?;
                let preset = self.project.preset(preset_id)?.clone();
                self.record_undo();
                for id in &ids {
                    let mut next = self.project.photo(*id)?.adjustments.clone();
                    next.apply_preset(&preset.adjustments);
                    self.commit_photo_adjustments(*id, format!("Preset: {}", preset.name), next)?;
                }
                Ok(CommandOutput::success(
                    "apply-preset",
                    format!("applied preset '{}' to {} photo(s)", preset.name, ids.len()),
                    ids,
                ))
            }
            Command::DeletePreset { id } => {
                self.project.preset(id)?;
                self.record_undo();
                self.project.delete_preset(id)?;
                Ok(CommandOutput::success(
                    "delete-preset",
                    format!("deleted preset {id}"),
                    vec![],
                ))
            }
            Command::CopyEdits { id } => {
                self.clipboard = Some(self.project.photo(id)?.adjustments.clone());
                Ok(CommandOutput::success(
                    "copy-edits",
                    format!("copied edits from photo {id}"),
                    vec![id],
                ))
            }
            Command::PasteEdits { ids } => {
                let adjustments = self
                    .clipboard
                    .clone()
                    .ok_or_else(|| anyhow::anyhow!("edit clipboard is empty"))?;
                self.ensure_ids(&ids)?;
                self.record_undo();
                for id in &ids {
                    self.commit_photo_adjustments(*id, "Paste edits", adjustments.clone())?;
                }
                Ok(CommandOutput::success("paste-edits", "pasted edits", ids))
            }
            Command::Remove { ids } => {
                self.ensure_ids(&ids)?;
                self.record_undo();
                self.project.photos.retain(|photo| !ids.contains(&photo.id));
                self.project.prune_empty_batches();
                if self.project.selected.is_some_and(|id| ids.contains(&id)) {
                    self.project.selected = self.project.photos.first().map(|photo| photo.id);
                }
                Ok(CommandOutput::success(
                    "remove",
                    "removed photos from catalog; original files were not changed",
                    ids,
                ))
            }
            Command::Rotate { id, clockwise } => {
                let mut next = self.project.photo(id)?.adjustments.clone();
                next.rotation = (next.rotation + if clockwise { 90 } else { -90 }).rem_euclid(360);
                self.record_undo();
                self.commit_photo_adjustments(id, "Rotate", next)?;
                Ok(CommandOutput::success(
                    "rotate",
                    format!("rotated photo {id}"),
                    vec![id],
                ))
            }
            Command::FlipHorizontal { id } => {
                let mut next = self.project.photo(id)?.adjustments.clone();
                next.flip_horizontal = !next.flip_horizontal;
                self.record_undo();
                self.commit_photo_adjustments(id, "Flip horizontal", next)?;
                Ok(CommandOutput::success(
                    "flip-horizontal",
                    format!("flipped photo {id}"),
                    vec![id],
                ))
            }
            Command::FlipVertical { id } => {
                let mut next = self.project.photo(id)?.adjustments.clone();
                next.flip_vertical = !next.flip_vertical;
                self.record_undo();
                self.commit_photo_adjustments(id, "Flip vertical", next)?;
                Ok(CommandOutput::success(
                    "flip-vertical",
                    format!("flipped photo {id}"),
                    vec![id],
                ))
            }
            Command::Export {
                id,
                path,
                max_size,
                quality,
            } => {
                export_photo(
                    self.project.photo(id)?,
                    &path,
                    RenderOptions { max_size },
                    quality,
                )?;
                Ok(CommandOutput::success(
                    "export",
                    format!("exported {}", path.display()),
                    vec![id],
                ))
            }
            Command::ExportBatch {
                ids,
                directory,
                format,
                max_size,
                quality,
            } => {
                self.ensure_ids(&ids)?;
                std::fs::create_dir_all(&directory)?;
                for id in &ids {
                    let photo = self.project.photo(*id)?;
                    export_photo(
                        photo,
                        &batch_destination(photo, &directory, format),
                        RenderOptions { max_size },
                        quality,
                    )?;
                }
                Ok(CommandOutput::success(
                    "export-batch",
                    format!("exported {} photo(s) to {}", ids.len(), directory.display()),
                    ids,
                ))
            }
            Command::Undo => {
                let previous = self
                    .undo
                    .pop()
                    .ok_or_else(|| anyhow::anyhow!("nothing to undo"))?;
                self.redo
                    .push(std::mem::replace(&mut self.project, previous));
                Ok(CommandOutput::success(
                    "undo",
                    "undid the last catalog change",
                    vec![],
                ))
            }
            Command::Redo => {
                let next = self
                    .redo
                    .pop()
                    .ok_or_else(|| anyhow::anyhow!("nothing to redo"))?;
                self.undo.push(std::mem::replace(&mut self.project, next));
                Ok(CommandOutput::success(
                    "redo",
                    "redid the last catalog change",
                    vec![],
                ))
            }
        }
    }

    fn record_undo(&mut self) {
        self.undo.push(self.project.clone());
        if self.undo.len() > 100 {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    fn ensure_ids(&self, ids: &[u64]) -> Result<()> {
        if ids.is_empty() {
            bail!("at least one photo id is required");
        }
        for id in ids {
            self.project.photo(*id)?;
        }
        Ok(())
    }

    fn replay_commands(&self, command: &Command) -> Result<Vec<Command>> {
        if let Command::PasteEdits { ids } = command {
            let adjustments = self.clipboard.clone().context("edit clipboard is empty")?;
            return Ok(ids
                .iter()
                .map(|id| Command::SetAdjustments {
                    id: *id,
                    adjustments: adjustments.clone(),
                })
                .collect());
        }
        Ok(vec![command.clone()])
    }

    fn from_durable(path: &Path, durable: DurableCatalog, project: Project) -> Self {
        Self {
            project,
            catalog_path: Some(path.to_owned()),
            clipboard: None,
            durable: Some(durable),
            legacy_photo_history: false,
            undo: Vec::new(),
            redo: Vec::new(),
        }
    }

    fn undo(&mut self) -> Result<CommandOutput> {
        if let Some(durable) = &mut self.durable {
            let id = self
                .project
                .selected
                .context("select a photo before undoing")?;
            self.project = durable.undo(id)?;
            self.clipboard = None;
            return Ok(CommandOutput::success(
                "undo",
                format!("went back one edit on photo {id}"),
                vec![id],
            ));
        }
        self.execute_in_memory(Command::Undo)
    }

    fn redo(&mut self) -> Result<CommandOutput> {
        if let Some(durable) = &mut self.durable {
            let id = self
                .project
                .selected
                .context("select a photo before redoing")?;
            self.project = durable.redo(id)?;
            self.clipboard = None;
            return Ok(CommandOutput::success(
                "redo",
                format!("went forward one edit on photo {id}"),
                vec![id],
            ));
        }
        self.execute_in_memory(Command::Redo)
    }

    fn commit_photo_adjustments(
        &mut self,
        id: u64,
        label: impl Into<String>,
        adjustments: Adjustments,
    ) -> Result<()> {
        let photo = self.project.photo_mut(id)?;
        if self.legacy_photo_history {
            photo.commit_adjustments(label, adjustments);
        } else {
            photo.adjustments = adjustments.sanitized();
        }
        Ok(())
    }
}

pub(crate) fn apply_replay_command(project: &mut Project, command: Command) -> Result<()> {
    let before = project.clone();
    let mut workspace = Workspace::new(std::mem::take(project), None);
    workspace.legacy_photo_history = false;
    match workspace.execute_in_memory(command) {
        Ok(_) => {
            *project = workspace.project;
            Ok(())
        }
        Err(error) => {
            *project = before;
            Err(error)
        }
    }
}

#[cfg(test)]
#[path = "command_tests.rs"]
mod tests;
