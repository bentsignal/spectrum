//! Editing an image: every edit is a change to its adjustments, kept as a
//! durable revision so it can be undone, redone, or revisited.
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use spectrum_document::{FileRole, FileVisitor, Model, Step};

use crate::{AdjustmentPatch, Adjustments, Image};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum Command {
    /// Changes some adjustments, leaving the rest.
    Adjust {
        patch: AdjustmentPatch,
    },
    /// Replaces every adjustment, as when copying another image's edits.
    SetAdjustments {
        adjustments: Adjustments,
    },
    /// Removes every edit.
    Reset,
    Undo,
    Redo,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CommandOutput {
    pub ok: bool,
    pub action: String,
    pub message: String,
}

fn output(action: &str, message: &str) -> CommandOutput {
    CommandOutput {
        ok: true,
        action: action.into(),
        message: message.into(),
    }
}

/// Spectrum's image documents.
pub struct ImageModel;

/// An image, in memory or durably in its revision file.
pub type Workspace = spectrum_document::Workspace<ImageModel>;

impl Model for ImageModel {
    type Document = Image;
    type Command = Command;
    type Output = CommandOutput;

    const APPLICATION: &'static str = "spectrum.image";
    const NOUN: &'static str = "image";

    fn apply(image: &mut Image, command: Command) -> Result<CommandOutput> {
        match command {
            Command::Adjust { patch } => {
                let mut adjustments = image.adjustments.clone();
                patch.apply_to(&mut adjustments);
                image.adjustments = adjustments.sanitized();
                Ok(output("adjust", "Adjusted"))
            }
            Command::SetAdjustments { adjustments } => {
                image.adjustments = adjustments.sanitized();
                Ok(output("set_adjustments", "Applied edits"))
            }
            Command::Reset => {
                image.adjustments = Adjustments::default();
                Ok(output("reset", "Reset all edits"))
            }
            Command::Undo | Command::Redo => bail!("history is stepped by the workspace"),
        }
    }

    fn describe(output: &CommandOutput) -> String {
        output.message.clone()
    }

    fn stepped(step: Step) -> CommandOutput {
        match step {
            Step::Undo => output("undo", "Went back one edit"),
            Step::Redo => output("redo", "Went forward one edit"),
        }
    }

    fn step(command: &Command) -> Option<Step> {
        match command {
            Command::Undo => Some(Step::Undo),
            Command::Redo => Some(Step::Redo),
            _ => None,
        }
    }

    fn document_files(image: &mut Image, visit: &mut FileVisitor) -> Result<()> {
        visit(&mut image.path, FileRole::Content)
    }

    fn command_files(_: &mut Command, _: &mut FileVisitor) -> Result<()> {
        Ok(())
    }

    fn loaded(image: &mut Image) -> Result<()> {
        image.adjustments = std::mem::take(&mut image.adjustments).sanitized();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use spectrum_document::{Actor, ActorKind, SessionId};

    fn person() -> Actor {
        Actor {
            id: "person:test".into(),
            display_name: "Tester".into(),
            kind: ActorKind::Human,
        }
    }

    fn photo(directory: &std::path::Path) -> std::path::PathBuf {
        let path = directory.join("photo.png");
        image::RgbaImage::from_pixel(6, 4, image::Rgba([120, 90, 60, 255]))
            .save(&path)
            .unwrap();
        path
    }

    #[test]
    fn edits_are_revisions_and_the_source_is_kept_inside() {
        let root = tempfile::tempdir().unwrap();
        let source = photo(root.path());
        let path = root.path().join("image.spectrum");
        let session = SessionId::new();
        let mut workspace =
            Workspace::create(&path, Image::import(&source).unwrap(), person(), session).unwrap();
        workspace
            .execute(Command::Adjust {
                patch: serde_json::from_str(r#"{"exposure":1.0}"#).unwrap(),
            })
            .unwrap();
        workspace.execute(Command::Reset).unwrap();
        workspace.execute(Command::Undo).unwrap();
        assert_eq!(workspace.document.adjustments.exposure, 1.0);
        drop(workspace);
        std::fs::remove_file(&source).unwrap();

        let reopened = Workspace::open(&path, person(), session).unwrap();
        assert_eq!(reopened.document.adjustments.exposure, 1.0);
        assert_eq!((reopened.document.width, reopened.document.height), (6, 4));
        let rendered =
            crate::engine::render(&reopened.document, crate::engine::RenderOptions::default())
                .unwrap();
        assert_eq!((rendered.width(), rendered.height()), (6, 4));
    }

    #[test]
    fn settings_are_sanitized_and_unknown_types_refused() {
        let root = tempfile::tempdir().unwrap();
        let mut workspace = Workspace::new(Image::import(&photo(root.path())).unwrap());
        workspace
            .execute(Command::Adjust {
                patch: serde_json::from_str(r#"{"exposure":99.0}"#).unwrap(),
            })
            .unwrap();
        assert!(workspace.document.adjustments.exposure <= 5.0);
        let text = root.path().join("notes.txt");
        std::fs::write(&text, b"not an image").unwrap();
        assert!(Image::import(&text).is_err());
    }
}
