use std::path::PathBuf;

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

use crate::{
    AdjustmentPatch, Adjustments, Project,
    engine::{RenderOptions, export_photo},
};

/// Complete application command surface shared by the GUI and CLI.
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

/// In-memory session state. Catalog snapshots make undo/redo predictable and
/// keep mutations centralized behind [`Workspace::execute`].
pub struct Workspace {
    pub project: Project,
    pub catalog_path: Option<PathBuf>,
    pub clipboard: Option<Adjustments>,
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
            undo: Vec::new(),
            redo: Vec::new(),
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn execute(&mut self, command: Command) -> Result<CommandOutput> {
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
                let destination = path.or_else(|| self.catalog_path.clone());
                let Some(destination) = destination else {
                    bail!("a destination path is required for an unsaved catalog");
                };
                self.project.save(&destination)?;
                self.catalog_path = Some(destination.clone());
                Ok(CommandOutput::success(
                    "save",
                    format!("saved {}", destination.display()),
                    vec![],
                ))
            }
            Command::Import { paths } => {
                // Import into a clone so one bad file cannot leave a half-imported catalog.
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
            Command::Adjust { id, patch } => {
                self.project.photo(id)?;
                self.record_undo();
                patch.apply_to(&mut self.project.photo_mut(id)?.adjustments);
                Ok(CommandOutput::success(
                    "adjust",
                    format!("adjusted photo {id}"),
                    vec![id],
                ))
            }
            Command::SetAdjustments { id, adjustments } => {
                self.project.photo(id)?;
                self.record_undo();
                self.project.photo_mut(id)?.adjustments = adjustments.sanitized();
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
                    self.project.photo_mut(*id)?.adjustments = Adjustments::default();
                }
                Ok(CommandOutput::success("reset", "reset edits", ids))
            }
            Command::CopyEdits { id } => {
                self.clipboard = Some(self.project.photo(id)?.adjustments);
                Ok(CommandOutput::success(
                    "copy-edits",
                    format!("copied edits from photo {id}"),
                    vec![id],
                ))
            }
            Command::PasteEdits { ids } => {
                let adjustments = self
                    .clipboard
                    .ok_or_else(|| anyhow::anyhow!("edit clipboard is empty"))?;
                self.ensure_ids(&ids)?;
                self.record_undo();
                for id in &ids {
                    self.project.photo_mut(*id)?.adjustments = adjustments;
                }
                Ok(CommandOutput::success("paste-edits", "pasted edits", ids))
            }
            Command::Remove { ids } => {
                self.ensure_ids(&ids)?;
                self.record_undo();
                self.project.photos.retain(|photo| !ids.contains(&photo.id));
                if self.project.selected.is_some_and(|id| ids.contains(&id)) {
                    self.project.selected = self.project.photos.first().map(|photo| photo.id);
                }
                Ok(CommandOutput::success(
                    "remove",
                    "removed photos from catalog",
                    ids,
                ))
            }
            Command::Rotate { id, clockwise } => {
                self.project.photo(id)?;
                self.record_undo();
                let adjustment = &mut self.project.photo_mut(id)?.adjustments;
                adjustment.rotation =
                    (adjustment.rotation + if clockwise { 90 } else { -90 }).rem_euclid(360);
                Ok(CommandOutput::success(
                    "rotate",
                    format!("rotated photo {id}"),
                    vec![id],
                ))
            }
            Command::FlipHorizontal { id } => {
                self.project.photo(id)?;
                self.record_undo();
                let adjustment = &mut self.project.photo_mut(id)?.adjustments;
                adjustment.flip_horizontal = !adjustment.flip_horizontal;
                Ok(CommandOutput::success(
                    "flip-horizontal",
                    format!("flipped photo {id}"),
                    vec![id],
                ))
            }
            Command::FlipVertical { id } => {
                self.project.photo(id)?;
                self.record_undo();
                let adjustment = &mut self.project.photo_mut(id)?.adjustments;
                adjustment.flip_vertical = !adjustment.flip_vertical;
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
            Command::Undo => {
                let Some(previous) = self.undo.pop() else {
                    bail!("nothing to undo");
                };
                self.redo
                    .push(std::mem::replace(&mut self.project, previous));
                Ok(CommandOutput::success(
                    "undo",
                    "undid the last change",
                    vec![],
                ))
            }
            Command::Redo => {
                let Some(next) = self.redo.pop() else {
                    bail!("nothing to redo");
                };
                self.undo.push(std::mem::replace(&mut self.project, next));
                Ok(CommandOutput::success(
                    "redo",
                    "redid the last change",
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
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    use image::{Rgba, RgbaImage};

    use super::*;

    #[test]
    fn adjustment_commands_are_undoable() {
        let mut project = Project::new("test");
        project.photos.push(crate::Photo {
            id: 1,
            path: "test.jpg".into(),
            name: "test.jpg".into(),
            width: 1,
            height: 1,
            adjustments: Adjustments::default(),
        });
        let mut workspace = Workspace::new(project, None);
        workspace
            .execute(Command::Adjust {
                id: 1,
                patch: AdjustmentPatch {
                    exposure: Some(2.0),
                    ..Default::default()
                },
            })
            .unwrap();
        assert_eq!(
            workspace.project.photo(1).unwrap().adjustments.exposure,
            2.0
        );
        workspace.execute(Command::Undo).unwrap();
        assert_eq!(
            workspace.project.photo(1).unwrap().adjustments.exposure,
            0.0
        );
        workspace.execute(Command::Redo).unwrap();
        assert_eq!(
            workspace.project.photo(1).unwrap().adjustments.exposure,
            2.0
        );
    }

    #[test]
    fn failed_multi_import_is_transactional() {
        let directory = std::env::temp_dir().join(format!(
            "lumen-import-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&directory).unwrap();
        let valid = directory.join("valid.png");
        let invalid = directory.join("invalid.jpg");
        RgbaImage::from_pixel(2, 2, Rgba([20, 40, 60, 255]))
            .save(&valid)
            .unwrap();
        fs::write(&invalid, b"not an image").unwrap();

        let mut workspace = Workspace::default();
        let result = workspace.execute(Command::Import {
            paths: vec![valid, invalid],
        });
        assert!(result.is_err());
        assert!(workspace.project.photos.is_empty());
        assert!(!workspace.can_undo());
        fs::remove_dir_all(directory).unwrap();
    }
}
