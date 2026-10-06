//! Who sees and edits which revision of an asset.
//!
//! Every asset's history is a tree of revisions, like commits. The person
//! has one session per library and always edits from the revision they are
//! looking at; nothing an agent does moves them. An agent works in its own
//! session, starting from where the person is. Agents either use a session
//! chosen with `agent start` (working together or separately), or by
//! default work together with the person: the person follows the agent's
//! revisions until they make an edit of their own, which branches.
use anyhow::{Context, Result, bail};
use spectrum_canvas::{CanvasModel, Document};
use spectrum_document::{
    Collaboration, CollaborationMode, History, Model, RevisionId, SessionId, Workspace,
};
use spectrum_image::{Image, ImageModel};
use spectrum_library::{AssetId, AssetKind};
use std::path::Path;

use super::{Service, links};

impl Service {
    /// Edits as the agent session `session` instead of working together
    /// with the person. Only agents choose sessions.
    pub fn with_session(mut self, session: Option<SessionId>) -> Result<Self> {
        if session.is_some() && self.is_person() {
            bail!("the person's edits use their own session");
        }
        self.chosen = session;
        Ok(self)
    }

    fn is_person(&self) -> bool {
        self.session == self.person
    }

    /// The session whose view this service reads: the person's, an agent's
    /// chosen session, or the agent the person is following.
    fn reader<M: Model>(&self, path: &Path) -> Result<SessionId> {
        if self.is_person() {
            return Ok(self.person);
        }
        if let Some(session) = self.chosen {
            return Ok(session);
        }
        Ok(Workspace::<M>::following(path, self.person)?
            .map_or(self.person, |collaboration| collaboration.agent_session))
    }

    /// The session this service edits in, set up if it is new.
    fn writer<M: Model>(&self, path: &Path) -> Result<SessionId> {
        if self.is_person() {
            return Ok(self.person);
        }
        if let Some(session) = self.chosen {
            return Ok(session);
        }
        // Catch the person up with this agent's earlier work, or notice
        // that they have gone their own way, then continue (or start)
        // working together from where they are.
        Workspace::<M>::join(path, super::person(), self.person)?;
        Workspace::<M>::follow(path, self.person)?;
        if let Some(collaboration) = Workspace::<M>::following(path, self.person)? {
            return Ok(collaboration.agent_session);
        }
        Ok(Workspace::<M>::start_collaboration(
            path,
            Some(self.person),
            self.actor.clone(),
            CollaborationMode::Together,
        )?
        .agent_session)
    }

    fn workspace<M: Model>(&self, path: &Path, base: Option<RevisionId>) -> Result<Workspace<M>> {
        let mut workspace = if self.is_person() {
            Workspace::<M>::open_at(path, self.actor.clone(), self.person, base)?
        } else {
            Workspace::<M>::open_session(path, self.writer::<M>(path)?)?
        };
        if !self.is_person()
            && let Some(base) = base
        {
            workspace.move_to(base)?;
        }
        Ok(workspace)
    }

    /// A document as this person or agent sees it. The person first
    /// follows an agent they work together with when `follow` is set.
    pub(crate) fn read<M: Model>(
        &self,
        path: &Path,
        follow: bool,
    ) -> Result<(M::Document, RevisionId)> {
        if follow && self.is_person() {
            Workspace::<M>::follow(path, self.person)?;
        }
        Workspace::<M>::read_session(path, self.reader::<M>(path)?)
    }

    /// An image as this person or agent sees it.
    pub fn image(&self, id: AssetId) -> Result<Image> {
        Ok(self
            .read::<ImageModel>(&self.document(id, AssetKind::Image)?, false)?
            .0)
    }

    /// A canvas as this person or agent sees it, with its linked images
    /// resolved for drawing.
    pub fn canvas(&self, id: AssetId) -> Result<Document> {
        let mut document = self.saved_canvas(id)?;
        self.resolve(&mut document)?;
        Ok(document)
    }

    /// A canvas as this person or agent sees it, its linked images as stored.
    pub fn saved_canvas(&self, id: AssetId) -> Result<Document> {
        Ok(self
            .read::<CanvasModel>(&self.document(id, AssetKind::Canvas)?, false)?
            .0)
    }

    /// Opens an image for the person: first following an agent they work
    /// together with, then reading the revision they are on.
    pub fn image_view(&self, id: AssetId) -> Result<(Image, RevisionId)> {
        let path = self.document(id, AssetKind::Image)?;
        Workspace::<ImageModel>::follow(&path, self.person)?;
        Workspace::<ImageModel>::read_session(&path, self.person)
    }

    /// Opens a canvas for the person, as [`Self::image_view`] does.
    pub fn canvas_view(&self, id: AssetId) -> Result<(Document, RevisionId)> {
        let path = self.document(id, AssetKind::Canvas)?;
        Workspace::<CanvasModel>::follow(&path, self.person)?;
        Workspace::<CanvasModel>::read_session(&path, self.person)
    }

    /// Follows an agent the person works together with, returning the
    /// revision the person is on afterward.
    pub fn follow(&self, id: AssetId) -> Result<RevisionId> {
        let asset = self.library.get(id)?;
        let path = self.library.path(&asset)?;
        match asset.kind {
            AssetKind::Image => follow::<ImageModel>(&path, self.person),
            AssetKind::Canvas => follow::<CanvasModel>(&path, self.person),
            kind => bail!("{kind} assets have no history yet"),
        }
    }

    /// Edits an image. Each edit is one revision in its history.
    pub fn edit_image(
        &self,
        id: AssetId,
        commands: Vec<spectrum_image::Command>,
    ) -> Result<Vec<spectrum_image::CommandOutput>> {
        Ok(self.edit_image_from(id, None, commands)?.0)
    }

    /// Edits an image from `base` (where the session is, when `None`),
    /// returning the outputs and the revision the edits made.
    pub fn edit_image_from(
        &self,
        id: AssetId,
        base: Option<RevisionId>,
        commands: Vec<spectrum_image::Command>,
    ) -> Result<(Vec<spectrum_image::CommandOutput>, RevisionId)> {
        let path = self.document(id, AssetKind::Image)?;
        let mut workspace = self.workspace::<ImageModel>(&path, base)?;
        let outputs = commands
            .into_iter()
            .map(|command| workspace.execute(command))
            .collect::<Result<Vec<_>>>()?;
        if let Some(error) = workspace.pending_publish_error() {
            bail!("the edit was saved but not published: {error}");
        }
        self.previews.borrow_mut().remove(&id);
        Ok((
            outputs,
            workspace.revision().context("image has no history")?,
        ))
    }

    /// Edits a canvas. Consecutive edits apply together as one revision;
    /// undo and redo step on their own.
    pub fn edit_canvas(
        &mut self,
        id: AssetId,
        commands: Vec<spectrum_canvas::Command>,
    ) -> Result<Vec<spectrum_canvas::CommandOutput>> {
        Ok(self.edit_canvas_from(id, None, commands)?.0)
    }

    /// Edits a canvas from `base`, returning the outputs and the revision
    /// the edits made.
    pub fn edit_canvas_from(
        &mut self,
        id: AssetId,
        base: Option<RevisionId>,
        commands: Vec<spectrum_canvas::Command>,
    ) -> Result<(Vec<spectrum_canvas::CommandOutput>, RevisionId)> {
        let path = self.document(id, AssetKind::Canvas)?;
        let mut workspace = self.workspace::<CanvasModel>(&path, base)?;
        let mut outputs = Vec::with_capacity(commands.len());
        let mut batch = Vec::new();
        for command in commands {
            if CanvasModel::step(&command).is_some() || CanvasModel::transient(&command) {
                if !batch.is_empty() {
                    outputs.extend(workspace.execute_batch(std::mem::take(&mut batch))?);
                }
                outputs.push(workspace.execute(command)?);
            } else {
                batch.push(command);
            }
        }
        if !batch.is_empty() {
            outputs.extend(workspace.execute_batch(batch)?);
        }
        if let Some(error) = workspace.pending_publish_error() {
            bail!("the edit was saved but not published: {error}");
        }
        let revision = workspace.revision().context("canvas has no history")?;
        if self.is_person() {
            self.library.references(id, &links(&workspace.document))?;
        } else {
            drop(workspace);
            self.index_canvas(id)?;
        }
        Ok((outputs, revision))
    }

    /// The document an editor works on directly and the session it edits in.
    pub fn editor_target(
        &self,
        id: AssetId,
        kind: AssetKind,
    ) -> Result<(std::path::PathBuf, SessionId)> {
        let path = self.document(id, kind)?;
        let session = match kind {
            AssetKind::Image => self.writer::<ImageModel>(&path)?,
            AssetKind::Canvas => self.writer::<CanvasModel>(&path)?,
            kind => bail!("{kind} assets have no editor yet"),
        };
        Ok((path, session))
    }

    /// The document an editor reads directly and the session whose view it reads.
    pub fn reader_target(
        &self,
        id: AssetId,
        kind: AssetKind,
    ) -> Result<(std::path::PathBuf, SessionId)> {
        let path = self.document(id, kind)?;
        let session = match kind {
            AssetKind::Image => self.reader::<ImageModel>(&path)?,
            AssetKind::Canvas => self.reader::<CanvasModel>(&path)?,
            kind => bail!("{kind} assets have no editor yet"),
        };
        Ok((path, session))
    }

    /// Records which images a canvas uses, as the person sees it and, for
    /// an agent, as the agent does: an image either still uses counts.
    pub fn index_canvas(&mut self, id: AssetId) -> Result<()> {
        let path = self.document(id, AssetKind::Canvas)?;
        let (person, _) = Workspace::<CanvasModel>::read_session(&path, self.person)?;
        let mut used = links(&person);
        let reader = self.reader::<CanvasModel>(&path)?;
        if reader != self.person {
            let (agent, _) = Workspace::<CanvasModel>::read_session(&path, reader)?;
            used.extend(
                links(&agent)
                    .into_iter()
                    .map(|(slot, image)| (format!("{reader}/{slot}"), image)),
            );
        }
        self.library.references(id, &used)
    }

    /// An asset's history: every revision, every session, and where this
    /// person or agent is.
    pub fn history(&self, id: AssetId) -> Result<History> {
        let asset = self.library.get(id)?;
        let path = self.library.path(&asset)?;
        match asset.kind {
            AssetKind::Image => {
                Workspace::<ImageModel>::history_of(&path, self.reader::<ImageModel>(&path)?)
            }
            AssetKind::Canvas => {
                Workspace::<CanvasModel>::history_of(&path, self.reader::<CanvasModel>(&path)?)
            }
            kind => bail!("{kind} assets have no history yet"),
        }
    }

    /// Moves this person or agent to another revision of an asset. Their
    /// next edit continues from there, branching if it has later work.
    pub fn move_to(&mut self, id: AssetId, revision: RevisionId) -> Result<()> {
        let asset = self.library.get(id)?;
        let path = self.library.path(&asset)?;
        match asset.kind {
            AssetKind::Image => {
                self.workspace::<ImageModel>(&path, Some(revision))?;
                self.previews.borrow_mut().remove(&id);
            }
            AssetKind::Canvas => {
                self.workspace::<CanvasModel>(&path, Some(revision))?;
                self.index_canvas(id)?;
            }
            kind => bail!("{kind} assets have no history yet"),
        }
        Ok(())
    }

    /// Starts an agent session on an asset from where the person is.
    /// Together, the person follows the agent until they edit; separately,
    /// the person never moves.
    pub fn start_agent(
        &self,
        id: AssetId,
        mode: CollaborationMode,
        name: Option<&str>,
    ) -> Result<Collaboration> {
        if self.is_person() {
            bail!("agent sessions are started by agents");
        }
        let mut actor = self.actor.clone();
        if let Some(name) = name.map(str::trim).filter(|name| !name.is_empty()) {
            actor.display_name = name.into();
        }
        let asset = self.library.get(id)?;
        let path = self.library.path(&asset)?;
        match asset.kind {
            AssetKind::Image => start::<ImageModel>(&path, self.person, actor, mode),
            AssetKind::Canvas => start::<CanvasModel>(&path, self.person, actor, mode),
            kind => bail!("{kind} assets have no history yet"),
        }
    }

    /// An agent session's collaboration with the person on an asset.
    pub fn agent_status(&self, id: AssetId, session: SessionId) -> Result<Collaboration> {
        let asset = self.library.get(id)?;
        let path = self.library.path(&asset)?;
        match asset.kind {
            AssetKind::Image => Workspace::<ImageModel>::collaboration(&path, session),
            AssetKind::Canvas => Workspace::<CanvasModel>::collaboration(&path, session),
            kind => bail!("{kind} assets have no history yet"),
        }
    }

    /// Changes some of an image's adjustments.
    pub fn adjust(&self, id: AssetId, patch: spectrum_image::AdjustmentPatch) -> Result<()> {
        self.edit_image(id, vec![spectrum_image::Command::Adjust { patch }])
            .map(drop)
    }

    /// Replaces an image's whole adjustment set, including curves, HSL, and
    /// color grading, which patches do not cover.
    pub fn set_adjustments(
        &self,
        id: AssetId,
        adjustments: spectrum_image::Adjustments,
    ) -> Result<()> {
        self.edit_image(
            id,
            vec![spectrum_image::Command::SetAdjustments { adjustments }],
        )
        .map(drop)
    }

    /// Gives each target image all of the source image's edits, crop included.
    pub fn apply_edits(&self, from: AssetId, to: &[AssetId]) -> Result<()> {
        let adjustments = self.image(from)?.adjustments;
        for id in to.iter().filter(|id| **id != from) {
            self.set_adjustments(*id, adjustments.clone())?;
        }
        Ok(())
    }

    /// Steps an image's edit history back or forward.
    pub fn step_history(&self, id: AssetId, forward: bool) -> Result<()> {
        let command = if forward {
            spectrum_image::Command::Redo
        } else {
            spectrum_image::Command::Undo
        };
        self.edit_image(id, vec![command]).map(drop)
    }
}

fn follow<M: Model>(path: &Path, person: SessionId) -> Result<RevisionId> {
    Workspace::<M>::follow(path, person)?;
    Workspace::<M>::cursor_of(path, person)
}

fn start<M: Model>(
    path: &Path,
    person: SessionId,
    agent: spectrum_document::Actor,
    mode: CollaborationMode,
) -> Result<Collaboration> {
    Workspace::<M>::join(path, super::person(), person)?;
    Workspace::<M>::start_collaboration(path, Some(person), agent, mode)
}
