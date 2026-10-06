//! A document being edited: in memory, or durably in its revision file.
use std::path::Path;

use anyhow::{Context, Result, bail};
use spectrum_revisions::{
    Actor, Collaboration, CollaborationMode, CollaborationSync, RevisionId, SessionId,
};

use crate::{Durable, History, Model, Step};

/// How many in-memory edits can be undone.
const MEMORY_UNDO: usize = 100;

/// A document and the commands that edit it. In memory it keeps a short
/// undo stack; opened from a file, every edit is a revision there.
pub struct Workspace<M: Model> {
    pub document: M::Document,
    durable: Option<Durable<M>>,
    undo: Vec<M::Document>,
    redo: Vec<M::Document>,
}

impl<M: Model> Workspace<M> {
    /// A document in memory.
    pub fn new(document: M::Document) -> Self {
        Self {
            document,
            durable: None,
            undo: Vec::new(),
            redo: Vec::new(),
        }
    }

    fn durably(durable: Durable<M>, document: M::Document) -> Self {
        Self {
            document,
            durable: Some(durable),
            undo: Vec::new(),
            redo: Vec::new(),
        }
    }

    /// Creates a revision file at `path` holding `document`.
    pub fn create(
        path: &Path,
        document: M::Document,
        actor: Actor,
        session_id: SessionId,
    ) -> Result<Self> {
        let (durable, document) = Durable::create(path, &document, actor, session_id)?;
        Ok(Self::durably(durable, document))
    }

    /// Opens a revision file in `session_id`.
    pub fn open(path: &Path, actor: Actor, session_id: SessionId) -> Result<Self> {
        let (durable, document) = Durable::open(path, actor, session_id)?;
        Ok(Self::durably(durable, document))
    }

    /// Opens a revision file in `session_id`, at `base` when given: edits
    /// then continue from that revision, branching if it has later work.
    pub fn open_at(
        path: &Path,
        actor: Actor,
        session_id: SessionId,
        base: Option<RevisionId>,
    ) -> Result<Self> {
        let mut workspace = Self::open(path, actor, session_id)?;
        if let Some(base) = base {
            workspace.move_to(base)?;
        }
        Ok(workspace)
    }

    /// Opens an existing session in a revision file, such as an agent's.
    pub fn open_session(path: &Path, session_id: SessionId) -> Result<Self> {
        let (durable, document) = Durable::open_session(path, session_id)?;
        Ok(Self::durably(durable, document))
    }

    /// The newest document in a revision file.
    pub fn read(path: &Path) -> Result<M::Document> {
        Durable::<M>::read(path)
    }

    /// Adds `session` to a document at its newest revision, unless it is there.
    pub fn join(path: &Path, actor: Actor, session: SessionId) -> Result<()> {
        Durable::<M>::join(path, actor, session)
    }

    /// The document as `session` sees it, and the revision it sees.
    pub fn read_session(path: &Path, session: SessionId) -> Result<(M::Document, RevisionId)> {
        Durable::<M>::read_session(path, session)
    }

    /// Where `session` is in a document.
    pub fn cursor_of(path: &Path, session: SessionId) -> Result<RevisionId> {
        Durable::<M>::cursor_of(path, session)
    }

    /// The revision tree, with `session`'s place in it as the current one.
    pub fn history_of(path: &Path, session: SessionId) -> Result<History> {
        Durable::<M>::history_of(path, session)
    }

    /// Moves a person following an agent to the agent's newest revision.
    pub fn follow(path: &Path, person: SessionId) -> Result<CollaborationSync> {
        Durable::<M>::follow(path, person)
    }

    /// The agent a person is following on this document, if any.
    pub fn following(path: &Path, person: SessionId) -> Result<Option<Collaboration>> {
        Durable::<M>::following(path, person)
    }

    /// Where this workspace's session is in its history.
    pub fn revision(&self) -> Option<RevisionId> {
        self.durable.as_ref().map(Durable::cursor)
    }

    pub fn start_collaboration(
        path: &Path,
        source: Option<SessionId>,
        agent: Actor,
        mode: CollaborationMode,
    ) -> Result<Collaboration> {
        Durable::<M>::start_collaboration(path, source, agent, mode)
    }

    pub fn collaboration(path: &Path, agent_session: SessionId) -> Result<Collaboration> {
        Durable::<M>::collaboration(path, agent_session)
    }

    pub fn durable(&self) -> Option<&Durable<M>> {
        self.durable.as_ref()
    }

    pub fn is_durable(&self) -> bool {
        self.durable.is_some()
    }

    /// Runs one command; Undo and Redo step through history.
    pub fn execute(&mut self, command: M::Command) -> Result<M::Output> {
        if let Some(step) = M::step(&command) {
            self.step(step)?;
            return Ok(M::stepped(step));
        }
        if M::transient(&command) {
            return M::apply(&mut self.document, command);
        }
        self.execute_batch(vec![command])?
            .pop()
            .context("the command produced no result")
    }

    /// Runs commands as one edit: all apply, or none do.
    pub fn execute_batch(&mut self, mut commands: Vec<M::Command>) -> Result<Vec<M::Output>> {
        if commands.is_empty() {
            bail!("there is nothing to do");
        }
        if commands
            .iter()
            .any(|command| M::step(command).is_some() || M::transient(command))
        {
            bail!("undo, redo, and selection cannot be part of an edit");
        }
        let mut candidate = self.document.clone();
        let mut outputs = Vec::with_capacity(commands.len());
        let Some(durable) = &mut self.durable else {
            for command in commands {
                outputs.push(M::apply(&mut candidate, command)?);
            }
            if candidate != self.document {
                let before = std::mem::replace(&mut self.document, candidate);
                self.undo.push(before);
                if self.undo.len() > MEMORY_UNDO {
                    self.undo.remove(0);
                }
                self.redo.clear();
            }
            return Ok(outputs);
        };
        M::prepare(&self.document, &mut commands)?;
        let prepared = durable.prepare(commands)?;
        for command in prepared.apply.iter().cloned() {
            outputs.push(M::apply(&mut candidate, command)?);
        }
        if candidate == self.document {
            return Ok(outputs);
        }
        let label = match outputs.as_slice() {
            [only] => M::describe(only),
            many => format!("Applied {} edits", many.len()),
        };
        durable.commit(prepared, &candidate, label)?;
        self.document = candidate;
        Ok(outputs)
    }

    /// Steps back or forward through history.
    pub fn step(&mut self, step: Step) -> Result<()> {
        if let Some(durable) = &mut self.durable {
            self.document = match step {
                Step::Undo => durable.undo()?,
                Step::Redo => durable.redo()?,
            };
            return Ok(());
        }
        let (from, to) = match step {
            Step::Undo => (&mut self.undo, &mut self.redo),
            Step::Redo => (&mut self.redo, &mut self.undo),
        };
        let document = from.pop().with_context(|| match step {
            Step::Undo => "nothing to undo",
            Step::Redo => "nothing to redo",
        })?;
        to.push(std::mem::replace(&mut self.document, document));
        Ok(())
    }

    pub fn can_undo(&self) -> bool {
        self.durable
            .as_ref()
            .map_or(!self.undo.is_empty(), Durable::can_undo)
    }

    pub fn can_redo(&self) -> bool {
        self.durable
            .as_ref()
            .map_or(!self.redo.is_empty(), Durable::can_redo)
    }

    /// The document's history, if it is durable.
    pub fn history(&self) -> Result<Option<History>> {
        self.durable.as_ref().map(Durable::history).transpose()
    }

    /// Moves to a revision; false if already there.
    pub fn move_to(&mut self, target: RevisionId) -> Result<bool> {
        let durable = self
            .durable
            .as_mut()
            .context("only a saved document has history to move through")?;
        if durable.cursor() == target {
            return Ok(false);
        }
        self.document = durable.move_to(target)?;
        Ok(true)
    }

    /// Follows a person's newer revisions while working together.
    pub fn sync_together(&mut self) -> Result<CollaborationSync> {
        let durable = self
            .durable
            .as_mut()
            .context("only a saved document can be shared")?;
        let (sync, document) = durable.sync_together()?;
        if let Some(document) = document {
            self.document = document;
        }
        Ok(sync)
    }

    pub fn session_id(&self) -> Option<SessionId> {
        self.durable.as_ref().map(Durable::session_id)
    }

    pub fn checkpoint(&self) -> Result<()> {
        self.durable.as_ref().map_or(Ok(()), Durable::checkpoint)
    }

    pub fn pending_publish_error(&self) -> Option<String> {
        self.durable.as_ref()?.pending_publish_error()
    }
}
