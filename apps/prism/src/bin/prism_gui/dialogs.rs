use super::*;

const NEW_DOCUMENT_NAME_ID: &str = "prism-new-document-name";
const ADD_TEXT_CONTENT_ID: &str = "prism-add-text-content";
const RENAME_LAYER_ID: &str = "prism-rename-layer-name";

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

fn modal_text_input(
    ui: &mut egui::Ui,
    text: &mut String,
    id_source: &'static str,
    multiline: bool,
) {
    let id = egui::Id::new(id_source);
    let initialized_id = id.with("focus-initialized");
    let mut output = if multiline {
        egui::TextEdit::multiline(text).id(id).show(ui)
    } else {
        egui::TextEdit::singleline(text).id(id).show(ui)
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

fn reset_modal_text_input(context: &egui::Context, id_source: &'static str) {
    let initialized_id = egui::Id::new(id_source).with("focus-initialized");
    context.data_mut(|data| data.remove_temp::<bool>(initialized_id));
}

impl PrismApp {
    pub(super) fn dialogs(&mut self, context: &egui::Context) {
        if self.delete_confirmation.is_some() {
            self.delete_dialog(context);
        } else if self.rename_layer.is_some() {
            self.rename_dialog(context);
        } else if self.text_dialog.is_some() {
            self.text_dialog(context);
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
            .collapsible(false)
            .resizable(false)
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
                ui.add_space(8.0);
                match modal_action(ui) {
                    ModalAction::Cancel => keep_open = false,
                    ModalAction::Confirm => {
                        create = true;
                        keep_open = false;
                    }
                    ModalAction::None => {}
                }
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        keep_open = false;
                    }
                    if ui
                        .button(RichText::new("Create canvas").color(ACCENT))
                        .clicked()
                    {
                        create = true;
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

    fn text_dialog(&mut self, context: &egui::Context) {
        let Some((position, mut text, mut size)) = self.text_dialog.take() else {
            return;
        };
        let mut insert = false;
        let mut keep_open = true;
        egui::Window::new("Add text")
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .show(context, |ui| {
                modal_text_input(ui, &mut text, ADD_TEXT_CONTENT_ID, true);
                ui.add(egui::Slider::new(&mut size, 8.0..=400.0).text("Size"));
                match modal_action(ui) {
                    ModalAction::Cancel => keep_open = false,
                    ModalAction::Confirm => {
                        insert = true;
                        keep_open = false;
                    }
                    ModalAction::None => {}
                }
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        keep_open = false;
                    }
                    if ui.button(RichText::new("Add text").color(ACCENT)).clicked() {
                        insert = true;
                        keep_open = false;
                    }
                });
            });
        if !keep_open {
            reset_modal_text_input(context, ADD_TEXT_CONTENT_ID);
        }
        if insert {
            self.execute(Command::AddText {
                text,
                name: None,
                font_size: size,
                color: [245, 246, 250, 255],
                x: position.x,
                y: position.y,
            });
            self.tool = Tool::Move;
        } else if keep_open {
            self.text_dialog = Some((position, text, size));
        }
    }

    fn rename_dialog(&mut self, context: &egui::Context) {
        let Some((id, mut name)) = self.rename_layer.take() else {
            return;
        };
        let mut save = false;
        let mut keep_open = true;
        egui::Window::new("Rename layer")
            .collapsible(false)
            .resizable(false)
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
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        keep_open = false;
                    }
                    if ui.button("Rename").clicked() {
                        save = true;
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
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .show(context, |ui| {
                ui.label("This removes the layer from the Prism document.");
                ui.label(
                    RichText::new("Linked source image files are never deleted.").color(ACCENT),
                );
                match modal_action(ui) {
                    ModalAction::Cancel => cancel = true,
                    ModalAction::Confirm => delete = true,
                    ModalAction::None => {}
                }
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                    if ui
                        .button(RichText::new("Delete layer").color(DANGER))
                        .clicked()
                    {
                        delete = true;
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
}
