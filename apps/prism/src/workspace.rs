use super::*;

pub struct Workspace {
    pub document: Document,
    pub project_path: Option<PathBuf>,
    durable: Option<DurableProject>,
    undo: Vec<Document>,
    redo: Vec<Document>,
    dirty: bool,
    interaction_before: Option<Document>,
    interaction_commands: Vec<Command>,
}

impl Default for Workspace {
    fn default() -> Self {
        Self::new(Document::default(), None)
    }
}

impl Workspace {
    pub fn new(document: Document, project_path: Option<PathBuf>) -> Self {
        Self {
            document,
            project_path,
            durable: None,
            undo: Vec::new(),
            redo: Vec::new(),
            dirty: false,
            interaction_before: None,
            interaction_commands: Vec::new(),
        }
    }

    pub fn open(path: &Path) -> Result<Self> {
        Self::open_as(
            path,
            spectrum_revisions::Actor {
                id: "local:human".into(),
                display_name: "Local User".into(),
                kind: spectrum_revisions::ActorKind::Human,
            },
            spectrum_revisions::SessionId::new(),
        )
    }

    pub fn load_read_only(path: &Path) -> Result<Document> {
        if DurableProject::looks_durable(path)? {
            return DurableProject::load_current(path);
        }
        load_document(path)
    }

    pub fn open_as(
        path: &Path,
        actor: spectrum_revisions::Actor,
        session_id: spectrum_revisions::SessionId,
    ) -> Result<Self> {
        if DurableProject::looks_durable(path)? {
            let (durable, document) = DurableProject::open(path, actor, session_id)?;
            return Ok(Self::from_durable(path, durable, document));
        }
        Ok(Self::new(load_document(path)?, Some(path.to_owned())))
    }

    pub fn open_session(path: &Path, session_id: spectrum_revisions::SessionId) -> Result<Self> {
        if !DurableProject::looks_durable(path)? {
            bail!("agent sessions require a durable Prism project");
        }
        let (durable, document) = DurableProject::open_session(path, session_id)?;
        Ok(Self::from_durable(path, durable, document))
    }

    pub fn start_collaboration(
        path: &Path,
        source_session: Option<spectrum_revisions::SessionId>,
        agent: spectrum_revisions::Actor,
        mode: spectrum_revisions::CollaborationMode,
    ) -> Result<spectrum_revisions::Collaboration> {
        DurableProject::start_collaboration(path, source_session, agent, mode)
    }

    pub fn collaboration(
        path: &Path,
        agent_session: spectrum_revisions::SessionId,
    ) -> Result<spectrum_revisions::Collaboration> {
        DurableProject::collaboration(path, agent_session)
    }

    pub fn create_durable(
        document: Document,
        path: &Path,
        actor: spectrum_revisions::Actor,
        session_id: spectrum_revisions::SessionId,
    ) -> Result<Self> {
        let (durable, document) = DurableProject::create(path, &document, actor, session_id)?;
        Ok(Self::from_durable(path, durable, document))
    }

    pub fn is_dirty(&self) -> bool {
        self.durable.is_none() && self.dirty
    }

    pub fn pending_publish_error(&self) -> Option<String> {
        self.durable
            .as_ref()
            .and_then(DurableProject::pending_publish_error)
    }

    pub fn session_id(&self) -> Option<spectrum_revisions::SessionId> {
        self.durable.as_ref().map(DurableProject::session_id)
    }

    pub fn checkpoint(&self) -> Result<()> {
        if let Some(durable) = &self.durable {
            durable.checkpoint()?;
        }
        Ok(())
    }

    pub fn history(&self) -> Result<Option<ProjectHistory>> {
        self.durable
            .as_ref()
            .map(DurableProject::history)
            .transpose()
    }

    pub fn move_to_revision(&mut self, target: spectrum_revisions::RevisionId) -> Result<bool> {
        if self.interaction_before.is_some() {
            bail!("finish the active interaction before navigating history");
        }
        let durable = self
            .durable
            .as_mut()
            .context("legacy Prism projects do not have a revision tree")?;
        if durable.cursor() == target {
            return Ok(false);
        }
        self.document = durable.move_to(target)?;
        self.dirty = false;
        Ok(true)
    }

    pub fn sync_together(&mut self) -> Result<spectrum_revisions::CollaborationSync> {
        if self.interaction_before.is_some() {
            return Ok(spectrum_revisions::CollaborationSync::Idle);
        }
        let Some(durable) = &mut self.durable else {
            return Ok(spectrum_revisions::CollaborationSync::Idle);
        };
        let (sync, document) = durable.sync_together()?;
        if let Some(document) = document {
            self.document = document;
            self.dirty = false;
        }
        Ok(sync)
    }

    pub fn can_undo(&self) -> bool {
        self.durable
            .as_ref()
            .map_or_else(|| !self.undo.is_empty(), DurableProject::can_undo)
    }

    pub fn can_redo(&self) -> bool {
        self.durable
            .as_ref()
            .map_or_else(|| !self.redo.is_empty(), DurableProject::can_redo)
    }

    pub fn save(&mut self, path: Option<&Path>) -> Result<PathBuf> {
        if let Some(durable) = &self.durable {
            if path.is_some_and(|path| Some(path) != self.project_path.as_deref()) {
                bail!("Save As for durable Prism projects is not implemented yet");
            }
            durable.checkpoint()?;
            return self
                .project_path
                .clone()
                .context("durable project path is missing");
        }
        if self.project_path.is_none()
            && let Some(path) = path
        {
            let (durable, document) = DurableProject::create(
                path,
                &self.document,
                spectrum_revisions::Actor {
                    id: "local:human".into(),
                    display_name: "Local User".into(),
                    kind: spectrum_revisions::ActorKind::Human,
                },
                spectrum_revisions::SessionId::new(),
            )?;
            *self = Self::from_durable(path, durable, document);
            return Ok(path.to_owned());
        }
        let path = path
            .map(Path::to_owned)
            .or_else(|| self.project_path.clone())
            .context("choose a .prism project path first")?;
        save_document(&self.document, &path)?;
        self.document = load_document(&path)?;
        self.project_path = Some(path.clone());
        self.dirty = false;
        Ok(path)
    }

    pub fn move_project(&mut self, destination: &Path) -> Result<PathBuf> {
        let source = self
            .project_path
            .clone()
            .context("this Prism project does not have a durable location")?;
        if source == destination {
            return Ok(source);
        }
        if destination.exists() {
            bail!(
                "refusing to replace existing project {}",
                destination.display()
            );
        }
        if let Some(parent) = destination
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent)?;
        }

        let durable = self
            .durable
            .take()
            .context("legacy Prism projects must be converted before they can be moved")?;
        durable.checkpoint()?;
        let actor = durable.actor().clone();
        let session_id = durable.session_id();
        drop(durable);

        let transfer = (|| -> Result<bool> {
            match fs::rename(&source, destination) {
                Ok(()) => Ok(false),
                Err(error) if error.kind() == std::io::ErrorKind::CrossesDevices => {
                    fs::copy(&source, destination).with_context(|| {
                        format!(
                            "could not copy {} to {}",
                            source.display(),
                            destination.display()
                        )
                    })?;
                    fs::File::open(destination)?.sync_all()?;
                    Ok(true)
                }
                Err(error) => Err(error).with_context(|| {
                    format!(
                        "could not move {} to {}",
                        source.display(),
                        destination.display()
                    )
                }),
            }
        })();
        let copied = match transfer {
            Ok(copied) => copied,
            Err(error) => {
                if source.exists() {
                    let _ = fs::remove_file(destination);
                }
                let (durable, document) = DurableProject::open(&source, actor, session_id)?;
                self.durable = Some(durable);
                self.document = document;
                return Err(error);
            }
        };

        let opened = DurableProject::open(destination, actor.clone(), session_id);
        let (durable, document) = match opened {
            Ok(opened) => opened,
            Err(error) => {
                if copied {
                    let _ = fs::remove_file(destination);
                } else {
                    let _ = fs::rename(destination, &source);
                }
                let (durable, document) = DurableProject::open(&source, actor, session_id)?;
                self.durable = Some(durable);
                self.document = document;
                return Err(error).context("moved Prism project could not be reopened");
            }
        };
        self.durable = Some(durable);
        self.document = document;
        self.project_path = Some(destination.to_owned());
        if copied {
            fs::remove_file(&source).with_context(|| {
                format!(
                    "project moved, but the old copy at {} could not be removed",
                    source.display()
                )
            })?;
        }
        Ok(destination.to_owned())
    }

    pub fn execute(&mut self, command: Command) -> Result<CommandOutput> {
        if self.interaction_before.is_some() {
            bail!("finish the active interaction before executing another command");
        }
        match command {
            Command::Undo => self.undo(),
            Command::Redo => self.redo(),
            command @ Command::SelectLayer { .. } => apply_command(&mut self.document, command),
            command => self
                .execute_batch(vec![command])?
                .pop()
                .context("command batch produced no output"),
        }
    }

    pub fn execute_batch(&mut self, commands: Vec<Command>) -> Result<Vec<CommandOutput>> {
        if self.interaction_before.is_some() {
            bail!("finish the active interaction before executing another command");
        }
        if commands.is_empty() {
            bail!("command batch is empty");
        }
        if commands.iter().any(|command| {
            matches!(
                command,
                Command::Undo | Command::Redo | Command::SelectLayer { .. }
            )
        }) {
            bail!("history and selection commands cannot be part of an edit batch");
        }
        let before = self.document.clone();
        let mut outputs = Vec::with_capacity(commands.len());
        for command in commands.iter().cloned() {
            match apply_command(&mut self.document, command) {
                Ok(output) => outputs.push(output),
                Err(error) => {
                    self.document = before;
                    return Err(error);
                }
            }
        }
        if self.document == before {
            return Ok(outputs);
        }
        if let Some(durable) = &mut self.durable {
            let label = if outputs.len() == 1 {
                outputs[0].message.clone()
            } else {
                format!("Applied {} actions", outputs.len())
            };
            if let Err(error) = durable.commit(&commands, &self.document, label) {
                self.document = before;
                return Err(error);
            }
            self.dirty = false;
        } else {
            self.record_edit(before);
        }
        Ok(outputs)
    }

    pub fn begin_interaction(&mut self) {
        if self.interaction_before.is_none() {
            self.interaction_before = Some(self.document.clone());
            self.interaction_commands.clear();
        }
    }

    pub fn preview(&mut self, command: Command) -> Result<CommandOutput> {
        self.preview_batch(vec![command])?
            .pop()
            .context("preview command batch produced no output")
    }

    pub fn preview_batch(&mut self, commands: Vec<Command>) -> Result<Vec<CommandOutput>> {
        if self.interaction_before.is_none() {
            bail!("begin an interaction before applying preview commands");
        }
        if commands.is_empty() {
            bail!("preview command batch is empty");
        }
        if commands.iter().any(|command| {
            matches!(
                command,
                Command::Undo | Command::Redo | Command::SelectLayer { .. }
            )
        }) {
            bail!("history and selection commands cannot preview an interaction");
        }
        let before = self.document.clone();
        let mut outputs = Vec::with_capacity(commands.len());
        for command in commands.iter().cloned() {
            match apply_command(&mut self.document, command) {
                Ok(output) => outputs.push(output),
                Err(error) => {
                    self.document = before;
                    return Err(error);
                }
            }
        }
        self.interaction_commands = commands;
        Ok(outputs)
    }

    pub fn commit_interaction(&mut self) -> Result<bool> {
        let Some(before) = self.interaction_before.take() else {
            return Ok(false);
        };
        let commands = std::mem::take(&mut self.interaction_commands);
        if self.document == before {
            return Ok(false);
        }
        if let Some(durable) = &mut self.durable {
            if commands.is_empty() {
                self.document = before;
                bail!("completed interaction has no semantic command");
            }
            if let Err(error) = durable.commit(
                &commands,
                &self.document,
                "Changed object with pointer gesture",
            ) {
                self.document = before;
                return Err(error);
            }
            self.dirty = false;
        } else {
            self.record_edit(before);
        }
        Ok(true)
    }

    pub fn cancel_interaction(&mut self) -> bool {
        let Some(before) = self.interaction_before.take() else {
            return false;
        };
        self.interaction_commands.clear();
        self.document = before;
        true
    }

    pub fn interaction_active(&self) -> bool {
        self.interaction_before.is_some()
    }

    fn from_durable(path: &Path, durable: DurableProject, document: Document) -> Self {
        Self {
            document,
            project_path: Some(path.to_owned()),
            durable: Some(durable),
            undo: Vec::new(),
            redo: Vec::new(),
            dirty: false,
            interaction_before: None,
            interaction_commands: Vec::new(),
        }
    }

    fn record_edit(&mut self, before: Document) {
        self.undo.push(before);
        if self.undo.len() > MAX_HISTORY {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.dirty = true;
    }

    fn undo(&mut self) -> Result<CommandOutput> {
        if let Some(durable) = &mut self.durable {
            self.document = durable.undo()?;
            self.dirty = false;
            return Ok(output("undo", "went back one edit", Vec::new()));
        }
        let previous = self.undo.pop().context("nothing to undo")?;
        self.redo
            .push(std::mem::replace(&mut self.document, previous));
        self.dirty = true;
        Ok(output("undo", "went back one edit", Vec::new()))
    }

    fn redo(&mut self) -> Result<CommandOutput> {
        if let Some(durable) = &mut self.durable {
            self.document = durable.redo()?;
            self.dirty = false;
            return Ok(output("redo", "went forward one edit", Vec::new()));
        }
        let next = self.redo.pop().context("nothing to redo")?;
        self.undo.push(std::mem::replace(&mut self.document, next));
        self.dirty = true;
        Ok(output("redo", "went forward one edit", Vec::new()))
    }
}
