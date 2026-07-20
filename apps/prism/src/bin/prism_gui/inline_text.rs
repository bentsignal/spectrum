use super::*;

const EDITOR_WIDTH: f32 = 360.0;
const EDITOR_HEIGHT: f32 = 142.0;
const EDITOR_GAP: f32 = 10.0;
const VIEWPORT_MARGIN: f32 = 8.0;

#[derive(Clone, Debug, PartialEq)]
pub(super) struct InlineTextEditor {
    tab_id: u64,
    layer_id: u64,
    text: String,
    font_size: f32,
    color: [u8; 4],
    error: Option<&'static str>,
    focus_requested: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StartError {
    Locked,
    NotText,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum EditorAction {
    #[default]
    None,
    Cancel,
    Done,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LifecycleAction {
    Commit,
    Cancel,
}

impl InlineTextEditor {
    fn start(tab_id: u64, layer: &Layer) -> Result<Self, StartError> {
        if layer.locked {
            return Err(StartError::Locked);
        }
        let LayerKind::Text {
            text,
            font_size,
            color,
            ..
        } = &layer.kind
        else {
            return Err(StartError::NotText);
        };
        Ok(Self {
            tab_id,
            layer_id: layer.id,
            text: text.clone(),
            font_size: *font_size,
            color: *color,
            error: None,
            focus_requested: false,
        })
    }

    fn update_command(&self) -> Result<Command, &'static str> {
        if self.text.trim().is_empty() {
            return Err("Text cannot be empty.");
        }
        Ok(Command::UpdateText {
            id: self.layer_id,
            text: self.text.clone(),
            font_size: self.font_size,
            color: self.color,
        })
    }

    fn owns(&self, tab_id: u64, layer_id: u64) -> bool {
        self.tab_id == tab_id && self.layer_id == layer_id
    }
}

pub(super) fn canvas_gestures_allowed(editor: Option<&InlineTextEditor>) -> bool {
    editor.is_none()
}

fn editor_action(
    escape_pressed: bool,
    enter_pressed: bool,
    command_modifier: bool,
    done_clicked: bool,
    cancel_clicked: bool,
) -> EditorAction {
    if escape_pressed || cancel_clicked {
        EditorAction::Cancel
    } else if done_clicked || enter_pressed && command_modifier {
        EditorAction::Done
    } else {
        EditorAction::None
    }
}

fn take_for_workspace_change(
    editor: &mut Option<InlineTextEditor>,
    active_tab_id: u64,
) -> Option<(u64, LifecycleAction)> {
    let editor = editor.take()?;
    if editor.tab_id != active_tab_id {
        return None;
    }
    let action = if editor.update_command().is_ok() {
        LifecycleAction::Commit
    } else {
        LifecycleAction::Cancel
    };
    Some((editor.layer_id, action))
}

fn transformed_visual_screen_bounds(
    geometry: CanvasGeometry,
    layer: &Layer,
    source_geometry: Option<LayerSourceGeometry>,
) -> Option<Rect> {
    let corners = rotated_layer_corners(layer, source_geometry)?;
    let first = geometry.canvas_to_screen(corners[0]);
    let mut bounds = Rect::from_min_max(first, first);
    for corner in corners.into_iter().skip(1) {
        bounds.extend_with(geometry.canvas_to_screen(corner));
    }
    Some(bounds)
}

fn editor_anchor(viewport: Rect, visual_bounds: Rect) -> Pos2 {
    let preferred_x = visual_bounds.center().x - EDITOR_WIDTH * 0.5;
    let preferred_y =
        if visual_bounds.top() - EDITOR_GAP - EDITOR_HEIGHT >= viewport.top() + VIEWPORT_MARGIN {
            visual_bounds.top() - EDITOR_GAP - EDITOR_HEIGHT
        } else {
            visual_bounds.bottom() + EDITOR_GAP
        };
    Pos2::new(
        preferred_x.clamp(
            viewport.left() + VIEWPORT_MARGIN,
            (viewport.right() - EDITOR_WIDTH - VIEWPORT_MARGIN)
                .max(viewport.left() + VIEWPORT_MARGIN),
        ),
        preferred_y.clamp(
            viewport.top() + VIEWPORT_MARGIN,
            (viewport.bottom() - EDITOR_HEIGHT - VIEWPORT_MARGIN)
                .max(viewport.top() + VIEWPORT_MARGIN),
        ),
    )
}

impl PrismApp {
    pub(super) fn open_text_editor(&mut self, id: u64) -> bool {
        if self
            .inline_text_editor
            .as_ref()
            .is_some_and(|editor| editor.owns(self.active_tab_id, id))
        {
            if let Some(editor) = self.inline_text_editor.as_mut() {
                editor.focus_requested = false;
            }
            return true;
        }
        let editor = match self.workspace.document.layer(id) {
            Ok(layer) => match InlineTextEditor::start(self.active_tab_id, layer) {
                Ok(editor) => editor,
                Err(StartError::Locked) => {
                    self.status = "Unlock the focused text before editing it.".into();
                    self.status_error = true;
                    return false;
                }
                Err(StartError::NotText) => return false,
            },
            Err(_) => return false,
        };
        self.settle_inline_text_editor();
        self.finish_interaction();
        self.workspace.begin_interaction();
        self.inline_text_editor = Some(editor);
        self.status = "Editing text · Command-Enter to finish · Escape to cancel".into();
        self.status_error = false;
        true
    }

    pub(super) fn settle_inline_text_editor(&mut self) {
        let Some((layer_id, action)) =
            take_for_workspace_change(&mut self.inline_text_editor, self.active_tab_id)
        else {
            return;
        };
        match action {
            LifecycleAction::Commit => self.finish_interaction(),
            LifecycleAction::Cancel => self.cancel_inline_text_interaction(layer_id),
        }
    }

    fn cancel_inline_text_editor(&mut self) {
        let Some(editor) = self.inline_text_editor.take() else {
            return;
        };
        if editor.tab_id == self.active_tab_id {
            self.cancel_inline_text_interaction(editor.layer_id);
        }
        self.status = "Canceled text edit".into();
        self.status_error = false;
    }

    fn cancel_inline_text_interaction(&mut self, layer_id: u64) {
        self.workspace.cancel_interaction();
        self.layer_visual_dirty.insert(layer_id);
    }

    fn finish_inline_text_editor(&mut self) -> bool {
        let Some(mut editor) = self.inline_text_editor.take() else {
            return false;
        };
        if let Err(error) = editor.update_command() {
            editor.error = Some(error);
            self.status = error.into();
            self.status_error = true;
            self.inline_text_editor = Some(editor);
            return false;
        }
        self.finish_interaction();
        true
    }

    pub(super) fn inline_text_editor_ui(
        &mut self,
        context: &egui::Context,
        geometry: CanvasGeometry,
    ) {
        let Some(mut editor) = self.inline_text_editor.take() else {
            return;
        };
        if editor.tab_id != self.active_tab_id {
            return;
        }
        let Some(visual_bounds) = self
            .workspace
            .document
            .layer(editor.layer_id)
            .ok()
            .and_then(|layer| transformed_visual_screen_bounds(geometry, layer, None))
        else {
            self.cancel_inline_text_interaction(editor.layer_id);
            return;
        };
        let anchor = editor_anchor(geometry.viewport, visual_bounds);
        let mut changed = false;
        let mut done_clicked = false;
        let mut cancel_clicked = false;
        let area_id = egui::Id::new(("prism-inline-text-editor", editor.tab_id, editor.layer_id));
        egui::Area::new(area_id)
            .order(egui::Order::Foreground)
            .fixed_pos(anchor)
            .movable(false)
            .show(context, |ui| {
                egui::Frame::new()
                    .fill(SURFACE)
                    .stroke(Stroke::new(1.0, ACCENT))
                    .corner_radius(RADIUS)
                    .inner_margin(egui::Margin::same(10))
                    .show(ui, |ui| {
                        ui.set_width(EDITOR_WIDTH - 20.0);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("EDIT TEXT").size(9.0).strong().color(ACCENT));
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.label(
                                        RichText::new("⌘↵ Done · Esc Cancel")
                                            .size(9.0)
                                            .color(MUTED),
                                    );
                                },
                            );
                        });
                        let response = egui::TextEdit::multiline(&mut editor.text)
                            .id(area_id.with("content"))
                            .desired_width(f32::INFINITY)
                            .desired_rows(3)
                            .show(ui)
                            .response;
                        if !editor.focus_requested {
                            response.request_focus();
                            editor.focus_requested = true;
                        }
                        changed = response.changed();
                        if let Some(error) = editor.error {
                            ui.label(RichText::new(error).size(10.0).color(DANGER));
                        }
                        ui.horizontal(|ui| {
                            cancel_clicked = quiet_button(ui, "Cancel").clicked();
                            done_clicked = primary_button(ui, "Done").clicked();
                        });
                    });
            });
        let action = context.input(|input| {
            editor_action(
                input.key_pressed(egui::Key::Escape),
                input.key_pressed(egui::Key::Enter),
                input.modifiers.command,
                done_clicked,
                cancel_clicked,
            )
        });
        if changed {
            match editor.update_command() {
                Ok(command) => {
                    editor.error = None;
                    if !self.preview_command(command) {
                        editor.error = Some("Prism could not preview this text.");
                    }
                }
                Err(error) => {
                    editor.error = Some(error);
                    self.status = error.into();
                    self.status_error = true;
                }
            }
        }
        self.inline_text_editor = Some(editor);
        match action {
            EditorAction::None => {}
            EditorAction::Cancel => self.cancel_inline_text_editor(),
            EditorAction::Done => {
                self.finish_inline_text_editor();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_layer(id: u64, locked: bool) -> Layer {
        Layer {
            id,
            locked,
            transform: Transform {
                x: 100.0,
                y: 80.0,
                rotation: 30.0,
                ..Default::default()
            },
            kind: LayerKind::Text {
                text: "Original".into(),
                font_size: 48.0,
                color: [10, 20, 30, 255],
                typography: prism_core::TextTypography::default(),
            },
            ..Default::default()
        }
    }

    fn workspace_with_text() -> Workspace {
        let mut document = Document::new("Inline", 800, 600);
        document.layers.push(text_layer(7, false));
        document.selected = Some(7);
        document.next_id = 8;
        Workspace::new(document, None)
    }

    fn layer_text(workspace: &Workspace) -> &str {
        let LayerKind::Text { text, .. } = &workspace.document.layer(7).unwrap().kind else {
            panic!("test layer should remain text");
        };
        text
    }

    #[test]
    fn locked_layers_are_rejected_and_unlocked_start_is_tab_and_target_scoped() {
        assert_eq!(
            InlineTextEditor::start(3, &text_layer(7, true)),
            Err(StartError::Locked)
        );
        let editor = InlineTextEditor::start(3, &text_layer(7, false)).unwrap();
        assert!(editor.owns(3, 7));
        assert!(!editor.owns(4, 7));
        assert!(!editor.owns(3, 8));
    }

    #[test]
    fn nonempty_changes_preserve_size_and_color() {
        let mut editor = InlineTextEditor::start(3, &text_layer(7, false)).unwrap();
        editor.text = "Changed".into();
        assert_eq!(
            editor.update_command().unwrap(),
            Command::UpdateText {
                id: 7,
                text: "Changed".into(),
                font_size: 48.0,
                color: [10, 20, 30, 255],
            }
        );
        editor.text = " \n".into();
        assert_eq!(editor.update_command(), Err("Text cannot be empty."));
    }

    #[test]
    fn cancel_restores_the_original_text_and_commit_records_one_revision() {
        let mut canceled = workspace_with_text();
        canceled.begin_interaction();
        canceled
            .preview(Command::UpdateText {
                id: 7,
                text: "Preview".into(),
                font_size: 48.0,
                color: [10, 20, 30, 255],
            })
            .unwrap();
        assert_eq!(layer_text(&canceled), "Preview");
        assert!(canceled.cancel_interaction());
        assert_eq!(layer_text(&canceled), "Original");
        assert!(!canceled.can_undo());

        let mut committed = workspace_with_text();
        committed.begin_interaction();
        for text in ["P", "Pr", "Preview"] {
            committed
                .preview(Command::UpdateText {
                    id: 7,
                    text: text.into(),
                    font_size: 48.0,
                    color: [10, 20, 30, 255],
                })
                .unwrap();
        }
        assert!(committed.commit_interaction().unwrap());
        assert!(committed.can_undo());
        committed.execute(Command::Undo).unwrap();
        assert_eq!(layer_text(&committed), "Original");
        assert!(committed.execute(Command::Undo).is_err());
    }

    #[test]
    fn lifecycle_cleanup_policy_never_carries_state_across_tabs() {
        let mut valid = Some(InlineTextEditor::start(3, &text_layer(7, false)).unwrap());
        assert_eq!(
            take_for_workspace_change(&mut valid, 3),
            Some((7, LifecycleAction::Commit))
        );
        assert!(valid.is_none());

        let mut empty = InlineTextEditor::start(3, &text_layer(7, false)).unwrap();
        empty.text.clear();
        let mut empty = Some(empty);
        assert_eq!(
            take_for_workspace_change(&mut empty, 3),
            Some((7, LifecycleAction::Cancel))
        );
        assert!(empty.is_none());

        let mut stale = Some(InlineTextEditor::start(3, &text_layer(7, false)).unwrap());
        assert_eq!(take_for_workspace_change(&mut stale, 4), None);
        assert!(stale.is_none());
    }

    #[test]
    fn hud_anchor_tracks_rotated_visual_bounds_and_stays_in_the_viewport() {
        let geometry = CanvasGeometry {
            viewport: Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 700.0)),
            canvas: Rect::from_min_size(Pos2::new(50.0, 50.0), Vec2::new(800.0, 600.0)),
            pixels_per_point: 1.0,
        };
        let layer = text_layer(7, false);
        let source = LayerSourceGeometry {
            size: Vec2::new(220.0, 80.0),
            visual_bounds: Rect::from_min_size(Pos2::new(8.0, 12.0), Vec2::new(180.0, 52.0)),
        };
        let screen_bounds =
            transformed_visual_screen_bounds(geometry, &layer, Some(source)).unwrap();
        assert!(screen_bounds.width() > 180.0);
        assert!(screen_bounds.height() > 52.0);
        let anchor = editor_anchor(geometry.viewport, screen_bounds);
        assert!(anchor.x >= geometry.viewport.left() + VIEWPORT_MARGIN);
        assert!(anchor.y >= geometry.viewport.top() + VIEWPORT_MARGIN);
        assert!(anchor.x + EDITOR_WIDTH <= geometry.viewport.right() - VIEWPORT_MARGIN);
        assert!(anchor.y + EDITOR_HEIGHT <= geometry.viewport.bottom() - VIEWPORT_MARGIN);
    }

    #[test]
    fn active_editor_gates_canvas_gestures_and_ui_actions_are_explicit() {
        let editor = InlineTextEditor::start(3, &text_layer(7, false)).unwrap();
        assert!(!canvas_gestures_allowed(Some(&editor)));
        assert!(canvas_gestures_allowed(None));
        assert_eq!(
            editor_action(false, true, true, false, false),
            EditorAction::Done
        );
        assert_eq!(
            editor_action(true, true, true, true, false),
            EditorAction::Cancel
        );
        assert_eq!(
            editor_action(false, true, false, false, false),
            EditorAction::None
        );
    }
}
