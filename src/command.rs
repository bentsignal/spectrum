use std::path::PathBuf;

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

use crate::{
    AdjustmentPatch, Adjustments, PickState, Project,
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
            Command::Adjust { id, patch } => {
                let mut next = self.project.photo(id)?.adjustments.clone();
                patch.apply_to(&mut next);
                self.record_undo();
                self.project
                    .photo_mut(id)?
                    .commit_adjustments("Develop adjustments", next);
                Ok(CommandOutput::success(
                    "adjust",
                    format!("adjusted photo {id}"),
                    vec![id],
                ))
            }
            Command::SetAdjustments { id, adjustments } => {
                self.project.photo(id)?;
                self.record_undo();
                self.project
                    .photo_mut(id)?
                    .commit_adjustments("Develop adjustments", adjustments);
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
                    self.project
                        .photo_mut(*id)?
                        .commit_adjustments("Reset all edits", Adjustments::default());
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
                    let photo = self.project.photo_mut(*id)?;
                    let mut next = photo.adjustments.clone();
                    next.apply_preset(&preset.adjustments);
                    photo.commit_adjustments(format!("Preset: {}", preset.name), next);
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
                    self.project
                        .photo_mut(*id)?
                        .commit_adjustments("Paste edits", adjustments.clone());
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
                let mut next = self.project.photo(id)?.adjustments.clone();
                next.rotation = (next.rotation + if clockwise { 90 } else { -90 }).rem_euclid(360);
                self.record_undo();
                self.project
                    .photo_mut(id)?
                    .commit_adjustments("Rotate", next);
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
                self.project
                    .photo_mut(id)?
                    .commit_adjustments("Flip horizontal", next);
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
                self.project
                    .photo_mut(id)?
                    .commit_adjustments("Flip vertical", next);
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgba, RgbaImage};
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn adjustment_history_survives_navigation() {
        let mut project = Project::new("test");
        project.photos.push(crate::Photo::new(
            1,
            "test.jpg".into(),
            "test.jpg".into(),
            1,
            1,
        ));
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
        workspace.execute(Command::HistoryBack { id: 1 }).unwrap();
        assert_eq!(
            workspace.project.photo(1).unwrap().adjustments.exposure,
            0.0
        );
        workspace
            .execute(Command::HistoryForward { id: 1 })
            .unwrap();
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

    #[test]
    fn presets_apply_to_multiple_photos_without_geometry() {
        let mut project = Project::new("test");
        let mut first = crate::Photo::new(1, "one.jpg".into(), "one.jpg".into(), 1, 1);
        first.adjustments.exposure = 1.0;
        let mut second = crate::Photo::new(2, "two.jpg".into(), "two.jpg".into(), 1, 1);
        second.adjustments.rotation = 90;
        project.photos.extend([first, second]);
        let mut workspace = Workspace::new(project, None);
        workspace
            .execute(Command::SavePreset {
                name: "Bright".into(),
                from_id: 1,
            })
            .unwrap();
        workspace
            .execute(Command::ApplyPreset {
                preset_id: 1,
                ids: vec![2],
            })
            .unwrap();
        let second = workspace.project.photo(2).unwrap();
        assert_eq!(second.adjustments.exposure, 1.0);
        assert_eq!(second.adjustments.rotation, 90);
        assert_eq!(second.history.last().unwrap().label, "Preset: Bright");
    }

    #[test]
    fn pick_state_is_a_core_catalog_command() {
        let mut project = Project::new("test");
        project.photos.push(crate::Photo::new(
            1,
            "one.jpg".into(),
            "one.jpg".into(),
            1,
            1,
        ));
        let mut workspace = Workspace::new(project, None);
        workspace
            .execute(Command::SetPick {
                ids: vec![1],
                state: PickState::Keep,
            })
            .unwrap();
        assert_eq!(workspace.project.photo(1).unwrap().pick, PickState::Keep);
        assert!(workspace.can_undo());
    }
}
