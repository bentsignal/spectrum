//! Durable documents for every kind of Spectrum asset. An engine describes
//! its document and commands as a [`Model`]; this crate keeps the document's
//! history in a revision file (snapshots, command batches, and the files the
//! document uses, embedded by content), and gives the engine a [`Workspace`]
//! to run commands on, in memory or durably, with undo, redo, history, and
//! agent sessions. Images, canvases, and future video share it.

use std::path::PathBuf;

use anyhow::Result;
use serde::{Serialize, de::DeserializeOwned};

mod durable;
mod files;
mod workspace;

pub use durable::{Durable, History, remove};
pub use files::{FileRole, hashed_bytes, is_embedded_reference};
pub use spectrum_revisions::{
    Actor, ActorKind, Collaboration, CollaborationMode, CollaborationSync, IoStats, Revision,
    RevisionId, Session, SessionId, io_stats, local_session_id,
};
pub use workspace::Workspace;

/// Visits a file a document or command refers to, and may replace its path.
pub type FileVisitor<'a> = dyn FnMut(&mut PathBuf, FileRole) -> Result<()> + 'a;

/// A step through history.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Undo,
    Redo,
}

/// What an engine tells the durable layer about its documents.
pub trait Model: 'static {
    type Document: Clone + PartialEq + Serialize + DeserializeOwned;
    type Command: Clone + Serialize + DeserializeOwned;
    type Output;

    /// Stored in each file; files of another kind are refused.
    const APPLICATION: &'static str;
    /// The kind of document, for messages: "canvas", "image".
    const NOUN: &'static str;

    /// Applies one command.
    fn apply(document: &mut Self::Document, command: Self::Command) -> Result<Self::Output>;

    /// A short description of what a command did, for history labels.
    fn describe(output: &Self::Output) -> String;

    /// The output for stepping through history.
    fn stepped(step: Step) -> Self::Output;

    /// Undo and Redo, if the command is one; the workspace handles them.
    fn step(_command: &Self::Command) -> Option<Step> {
        None
    }

    /// Commands that only change in-memory state, such as which layer is
    /// selected; they apply without a revision.
    fn transient(_command: &Self::Command) -> bool {
        false
    }

    /// Visits every file the document refers to.
    fn document_files(document: &mut Self::Document, visit: &mut FileVisitor) -> Result<()>;

    /// Visits every file a command refers to.
    fn command_files(command: &mut Self::Command, visit: &mut FileVisitor) -> Result<()>;

    /// Readies commands for durable application against the document, such
    /// as resolving markers that refer to its current state.
    fn prepare(_document: &Self::Document, _commands: &mut [Self::Command]) -> Result<()> {
        Ok(())
    }

    /// The form a command is stored in, and whether its batch must also
    /// store a snapshot (a command whose result cannot be replayed).
    fn stored(command: &Self::Command) -> (Self::Command, bool) {
        (command.clone(), false)
    }

    /// Checks a command read from a file before it is replayed, refusing
    /// anything only an author may submit.
    fn check_stored(_command: &Self::Command) -> Result<()> {
        Ok(())
    }

    /// Checks a document read from a file before it is used.
    fn loaded(_document: &mut Self::Document) -> Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests;
