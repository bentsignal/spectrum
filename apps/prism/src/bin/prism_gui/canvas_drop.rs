use super::*;
use anyhow::{Context, bail};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct CanvasDropRoutingState {
    workspace_ready: bool,
    keyboard_focus: bool,
    inline_text_editing: bool,
    terminal_visible: bool,
    modal_open: bool,
    history_visible: bool,
    interaction_active: bool,
}

impl CanvasDropRoutingState {
    fn routes_to_canvas(self) -> bool {
        self.workspace_ready
            && !self.keyboard_focus
            && !self.inline_text_editing
            && !self.terminal_visible
            && !self.modal_open
            && !self.history_visible
            && !self.interaction_active
    }
}

impl PrismApp {
    pub(super) fn handle_canvas_file_drops(&mut self, ui: &egui::Ui, geometry: CanvasGeometry) {
        let routing = CanvasDropRoutingState {
            workspace_ready: self.workspace_initialized,
            keyboard_focus: ui.ctx().egui_wants_keyboard_input(),
            inline_text_editing: self.inline_text_editor.is_some(),
            terminal_visible: self.terminal.visible(),
            modal_open: self.has_modal_surface(),
            history_visible: self.history.visible,
            interaction_active: self.workspace.interaction_active(),
        };
        if !routing.routes_to_canvas() {
            return;
        }
        let (files, pointer) =
            ui.input(|input| (input.raw.dropped_files.clone(), input.pointer.latest_pos()));
        if files.is_empty() {
            return;
        }
        let Some(target) = canvas_drop_target(geometry, pointer) else {
            return;
        };
        let commands = match canvas_drop_commands(&files, target) {
            Ok(commands) => commands,
            Err(error) => {
                self.status = format!("Could not place dropped images: {error:#}");
                self.status_error = true;
                return;
            }
        };
        let count = commands.len();
        if self.execute_batch(commands) {
            self.status = format!("Placed {count} image{}", if count == 1 { "" } else { "s" });
            self.status_error = false;
        }
    }
}

fn canvas_drop_commands(files: &[egui::DroppedFile], target: Pos2) -> anyhow::Result<Vec<Command>> {
    let mut commands = Vec::with_capacity(files.len());
    for (index, file) in files.iter().enumerate() {
        let path = file.path.as_ref().with_context(|| {
            if file.name.is_empty() {
                "a dropped item is not a local file".to_owned()
            } else {
                format!("{} is not a local file", file.name)
            }
        })?;
        if !is_supported_image_drop(path) {
            bail!(
                "{} is not a supported JPG, PNG, TIFF, or WebP image",
                path.display()
            );
        }
        let (width, height) = image::image_dimensions(path)
            .with_context(|| format!("could not inspect {}", path.display()))?;
        let origin = raster_drop_origin(target, width, height) + Vec2::splat(index as f32 * 16.0);
        commands.push(Command::AddRaster {
            path: path.clone(),
            name: None,
            x: origin.x,
            y: origin.y,
        });
    }
    Ok(commands)
}

fn canvas_drop_target(geometry: CanvasGeometry, pointer: Option<Pos2>) -> Option<Pos2> {
    match pointer {
        Some(pointer) if geometry.viewport.contains(pointer) => {
            Some(geometry.screen_to_canvas(pointer))
        }
        Some(_) => None,
        None => Some(Pos2::new(
            geometry.canvas.width() / geometry.pixels_per_point * 0.5,
            geometry.canvas.height() / geometry.pixels_per_point * 0.5,
        )),
    }
}

fn is_supported_image_drop(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "jpg" | "jpeg" | "png" | "tif" | "tiff" | "webp"
            )
        })
}

fn raster_drop_origin(target: Pos2, width: u32, height: u32) -> Pos2 {
    target - Vec2::new(width as f32 * 0.5, height as f32 * 0.5)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn drop_routing_requires_exclusive_idle_canvas_ownership() {
        let ready = CanvasDropRoutingState {
            workspace_ready: true,
            ..Default::default()
        };
        assert!(ready.routes_to_canvas());
        for blocked in [
            CanvasDropRoutingState {
                workspace_ready: false,
                ..ready
            },
            CanvasDropRoutingState {
                keyboard_focus: true,
                ..ready
            },
            CanvasDropRoutingState {
                inline_text_editing: true,
                ..ready
            },
            CanvasDropRoutingState {
                terminal_visible: true,
                ..ready
            },
            CanvasDropRoutingState {
                modal_open: true,
                ..ready
            },
            CanvasDropRoutingState {
                history_visible: true,
                ..ready
            },
            CanvasDropRoutingState {
                interaction_active: true,
                ..ready
            },
        ] {
            assert!(!blocked.routes_to_canvas());
        }
    }

    #[test]
    fn finder_drop_classification_accepts_supported_images_case_insensitively() {
        for name in [
            "photo.JPG",
            "photo.jpeg",
            "art.png",
            "scan.TIFF",
            "layer.tif",
            "asset.webp",
        ] {
            assert!(is_supported_image_drop(Path::new(name)), "{name}");
        }
        for name in ["font.ttf", "document.pdf", "image.bmp", "no-extension"] {
            assert!(!is_supported_image_drop(Path::new(name)), "{name}");
        }
    }

    #[test]
    fn finder_drop_centers_image_at_pointer_in_document_coordinates() {
        let geometry = canvas_geometry(
            Rect::from_min_size(Pos2::new(20.0, 30.0), Vec2::new(1000.0, 700.0)),
            800,
            600,
            1.75,
            Vec2::new(47.0, -31.0),
        );
        let intended_center = Pos2::new(321.0, 234.0);
        let target =
            canvas_drop_target(geometry, Some(geometry.canvas_to_screen(intended_center))).unwrap();
        let origin = raster_drop_origin(target, 120, 80);
        assert!((origin.x - 261.0).abs() < 0.001);
        assert!((origin.y - 194.0).abs() < 0.001);
        assert_eq!(
            canvas_drop_target(geometry, Some(geometry.viewport.right_bottom() + Vec2::ONE)),
            None
        );
    }

    #[test]
    fn finder_drop_without_pointer_uses_canvas_center() {
        let geometry = canvas_geometry(
            Rect::from_min_size(Pos2::new(20.0, 30.0), Vec2::new(1000.0, 700.0)),
            800,
            600,
            1.75,
            Vec2::new(47.0, -31.0),
        );
        assert_eq!(
            canvas_drop_target(geometry, None),
            Some(Pos2::new(400.0, 300.0))
        );
    }

    #[test]
    fn multi_file_drop_executes_as_one_atomic_command_batch() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!("prism-drop-batch-{stamp}"));
        std::fs::create_dir_all(&directory).unwrap();
        let first = directory.join("first.png");
        let second = directory.join("second.png");
        image::RgbaImage::new(4, 3).save(&first).unwrap();
        image::RgbaImage::new(5, 2).save(&second).unwrap();
        let files = [
            egui::DroppedFile {
                path: Some(first),
                ..Default::default()
            },
            egui::DroppedFile {
                path: Some(second.clone()),
                ..Default::default()
            },
        ];
        let commands = canvas_drop_commands(&files, Pos2::new(100.0, 80.0)).unwrap();
        assert_eq!(commands.len(), 2);
        std::fs::remove_file(second).unwrap();
        let mut workspace = Workspace::new(Document::new("Drop batch", 320, 200), None);

        assert!(workspace.execute_batch(commands).is_err());
        assert!(workspace.document.layers.is_empty());
        assert!(!workspace.can_undo());
        std::fs::remove_dir_all(directory).unwrap();
    }
}
