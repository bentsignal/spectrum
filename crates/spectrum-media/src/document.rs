//! The library document for imported footage or audio. The file itself is
//! too large to embed in a revision file: it lives once in the library's
//! media store, named by its content, and the document records which file
//! it is and what it holds.
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use spectrum_document::{FileVisitor, Model, Step};

use crate::MediaInfo;

/// Imported footage or audio.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Media {
    /// The file in the library's media store, `<sha256>.<extension>`.
    pub file: String,
    /// The name the file had when it was imported.
    pub original_name: String,
    pub bytes: u64,
    pub info: MediaInfo,
}

/// Imported media has no edits yet; its history records the import.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum Command {
    Undo,
    Redo,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CommandOutput {
    pub ok: bool,
    pub action: String,
    pub message: String,
}

/// Spectrum's footage and audio documents.
pub struct MediaModel;

/// Footage or audio, in memory or durably in its revision file.
pub type Workspace = spectrum_document::Workspace<MediaModel>;

impl Model for MediaModel {
    type Document = Media;
    type Command = Command;
    type Output = CommandOutput;

    const APPLICATION: &'static str = "spectrum.media";
    const NOUN: &'static str = "media";

    fn apply(_: &mut Media, _: Command) -> Result<CommandOutput> {
        bail!("history is stepped by the workspace")
    }

    fn describe(output: &CommandOutput) -> String {
        output.message.clone()
    }

    fn stepped(step: Step) -> CommandOutput {
        let (action, message) = match step {
            Step::Undo => ("undo", "Went back one edit"),
            Step::Redo => ("redo", "Went forward one edit"),
        };
        CommandOutput {
            ok: true,
            action: action.into(),
            message: message.into(),
        }
    }

    fn step(command: &Command) -> Option<Step> {
        Some(match command {
            Command::Undo => Step::Undo,
            Command::Redo => Step::Redo,
        })
    }

    fn document_files(_: &mut Media, _: &mut FileVisitor) -> Result<()> {
        Ok(())
    }

    fn command_files(_: &mut Command, _: &mut FileVisitor) -> Result<()> {
        Ok(())
    }
}
