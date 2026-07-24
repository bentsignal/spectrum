use super::*;

const NEW_DOCUMENT_NAME_ID: &str = "prism-new-document-name";
const RENAME_LAYER_ID: &str = "prism-rename-layer-name";
pub(super) const RENAME_DOCUMENT_ID: &str = "prism-rename-document-name";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum ModalAction {
    #[default]
    None,
    Cancel,
    Confirm,
}

pub(super) fn modal_action(ui: &egui::Ui) -> ModalAction {
    ui.input(|input| {
        modal_action_from_keys(
            input.key_pressed(egui::Key::Escape),
            input.key_pressed(egui::Key::Enter),
            input.modifiers.shift,
        )
    })
}

fn modal_action_from_keys(
    escape_pressed: bool,
    enter_pressed: bool,
    shift_pressed: bool,
) -> ModalAction {
    if escape_pressed {
        ModalAction::Cancel
    } else if enter_pressed && !shift_pressed {
        ModalAction::Confirm
    } else {
        ModalAction::None
    }
}

pub(super) fn modal_text_input(
    ui: &mut egui::Ui,
    text: &mut String,
    id_source: &'static str,
    multiline: bool,
) {
    let id = egui::Id::new(id_source);
    let initialized_id = id.with("focus-initialized");
    let mut output = if multiline {
        egui::TextEdit::multiline(text)
            .id(id)
            .margin(egui::Margin::symmetric(
                CONTROL_HORIZONTAL_PADDING,
                CONTROL_VERTICAL_PADDING,
            ))
            .desired_width(f32::INFINITY)
            .desired_rows(5)
            .show(ui)
    } else {
        text_field(text)
            .id(id)
            .desired_width(f32::INFINITY)
            .show(ui)
    };
    let initialized = ui.data(|data| data.get_temp::<bool>(initialized_id).unwrap_or(false));
    if !initialized {
        output.response.request_focus();
        output
            .state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::two(
                egui::text::CCursor::default(),
                egui::text::CCursor::new(text.chars().count()),
            )));
        output.state.store(ui.ctx(), output.response.id);
        ui.data_mut(|data| data.insert_temp(initialized_id, true));
    }
}

pub(super) fn reset_modal_text_input(context: &egui::Context, id_source: &'static str) {
    let initialized_id = egui::Id::new(id_source).with("focus-initialized");
    context.data_mut(|data| data.remove_temp::<bool>(initialized_id));
}

const MODAL_BACKDROP_ALPHA: u8 = 148;

fn modal_backdrop_sense() -> Sense {
    Sense::click_and_drag()
}

fn show_modal_backdrop(context: &egui::Context) {
    let screen = context.content_rect();
    egui::Area::new(egui::Id::new("prism-modal-backdrop"))
        .order(egui::Order::Middle)
        .fixed_pos(screen.min)
        .movable(false)
        .interactable(true)
        .show(context, |ui| {
            ui.set_min_size(screen.size());
            let (rect, _) = ui.allocate_exact_size(screen.size(), modal_backdrop_sense());
            ui.painter()
                .rect_filled(rect, 0.0, Color32::from_black_alpha(MODAL_BACKDROP_ALPHA));
        });
}

fn modal_surface_present(states: [bool; 7]) -> bool {
    states.into_iter().any(|present| present)
}

impl PrismApp {
    pub(super) fn has_modal_surface(&self) -> bool {
        modal_surface_present([
            self.move_project_dialog.is_some(),
            self.delete_confirmation.is_some(),
            self.rename_layer.is_some(),
            self.rename_document.is_some(),
            self.new_dialog.is_some(),
            self.tool_palette.is_some(),
            self.shape_palette.is_some(),
        ])
    }

    pub(super) fn edit_focused(&mut self) {
        let Some(layer) = self.selected_layer().cloned() else {
            self.status = "Focus an object before editing it.".into();
            self.status_error = true;
            return;
        };
        if matches!(layer.kind, LayerKind::Text { .. }) {
            self.open_text_editor(layer.id);
        } else {
            self.status = "Object controls are ready in the Inspector.".into();
            self.status_error = false;
        }
    }

    pub(super) fn dialogs(&mut self, context: &egui::Context) {
        if self.has_modal_surface() {
            show_modal_backdrop(context);
        }
        if self.move_project_dialog.is_some() {
            self.move_project_dialog(context);
        } else if self.rename_document.is_some() {
            self.rename_document_dialog(context);
        } else if self.delete_confirmation.is_some() {
            self.delete_dialog(context);
        } else if self.rename_layer.is_some() {
            self.rename_dialog(context);
        } else if self.new_dialog.is_some() {
            self.new_document_dialog(context);
        } else {
            self.tool_palette_dialog(context);
        }
    }

    fn new_document_dialog(&mut self, context: &egui::Context) {
        let Some(mut draft) = self.new_dialog.take() else {
            return;
        };
        let mut create = false;
        let mut keep_open = true;
        egui::Window::new("New Prism document")
            .order(egui::Order::Foreground)
            .collapsible(false)
            .resizable(false)
            .fixed_size(Vec2::new(360.0, 152.0))
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .show(context, |ui| {
                ui.label("Name");
                modal_text_input(ui, &mut draft.name, NEW_DOCUMENT_NAME_ID, false);
                ui.horizontal(|ui| {
                    ui.label("Width");
                    ui.add(egui::DragValue::new(&mut draft.width).range(1..=32_768));
                    ui.label("Height");
                    ui.add(egui::DragValue::new(&mut draft.height).range(1..=32_768));
                });
                ui.add_space(6.0);
                match modal_action(ui) {
                    ModalAction::Cancel => keep_open = false,
                    ModalAction::Confirm => {
                        create = true;
                        keep_open = false;
                    }
                    ModalAction::None => {}
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if primary_button(ui, "Create canvas").clicked() {
                        create = true;
                        keep_open = false;
                    }
                    if quiet_button(ui, "Cancel").clicked() {
                        keep_open = false;
                    }
                });
            });
        if !keep_open {
            reset_modal_text_input(context, NEW_DOCUMENT_NAME_ID);
        }
        if create {
            self.new_document(draft);
        } else if keep_open {
            self.new_dialog = Some(draft);
        }
    }

    fn rename_dialog(&mut self, context: &egui::Context) {
        let Some((id, mut name)) = self.rename_layer.take() else {
            return;
        };
        let mut save = false;
        let mut keep_open = true;
        egui::Window::new("Rename layer")
            .order(egui::Order::Foreground)
            .collapsible(false)
            .resizable(false)
            .fixed_size(Vec2::new(360.0, 108.0))
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .show(context, |ui| {
                modal_text_input(ui, &mut name, RENAME_LAYER_ID, false);
                match modal_action(ui) {
                    ModalAction::Cancel => keep_open = false,
                    ModalAction::Confirm => {
                        save = true;
                        keep_open = false;
                    }
                    ModalAction::None => {}
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if primary_button(ui, "Rename").clicked() {
                        save = true;
                        keep_open = false;
                    }
                    if quiet_button(ui, "Cancel").clicked() {
                        keep_open = false;
                    }
                });
            });
        if !keep_open {
            reset_modal_text_input(context, RENAME_LAYER_ID);
        }
        if save {
            self.execute(Command::RenameLayer { id, name });
        } else if keep_open {
            self.rename_layer = Some((id, name));
        }
    }

    fn delete_dialog(&mut self, context: &egui::Context) {
        let Some(id) = self.delete_confirmation else {
            return;
        };
        let mut delete = false;
        let mut cancel = false;
        egui::Window::new("Delete layer?")
            .order(egui::Order::Foreground)
            .collapsible(false)
            .resizable(false)
            .fixed_size(Vec2::new(380.0, 126.0))
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .show(context, |ui| {
                ui.label("This removes the layer from the Prism document.");
                ui.label(
                    RichText::new("Linked source image files are never deleted.").color(MUTED),
                );
                match modal_action(ui) {
                    ModalAction::Cancel => cancel = true,
                    ModalAction::Confirm => delete = true,
                    ModalAction::None => {}
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if danger_button(ui, "Delete layer").clicked() {
                        delete = true;
                    }
                    if quiet_button(ui, "Cancel").clicked() {
                        cancel = true;
                    }
                });
            });
        if delete {
            self.delete_confirmation = None;
            self.execute(Command::RemoveLayer { id });
        }
        if cancel {
            self.delete_confirmation = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modal_keyboard_contract_is_consistent() {
        assert_eq!(
            modal_action_from_keys(true, false, false),
            ModalAction::Cancel
        );
        assert_eq!(
            modal_action_from_keys(false, true, false),
            ModalAction::Confirm
        );
        assert_eq!(modal_action_from_keys(false, true, true), ModalAction::None);
        assert_eq!(
            modal_action_from_keys(true, true, false),
            ModalAction::Cancel
        );
    }

    #[test]
    fn modal_backdrop_consumes_clicks_and_drags() {
        let sense = modal_backdrop_sense();
        assert!(sense.senses_click());
        assert!(sense.senses_drag());
        assert!(sense.interactive());
    }

    #[test]
    fn every_dialog_state_gates_the_shared_modal_surface() {
        assert!(!modal_surface_present([false; 7]));
        for index in 0..7 {
            let mut states = [false; 7];
            states[index] = true;
            assert!(modal_surface_present(states));
        }
    }
}
