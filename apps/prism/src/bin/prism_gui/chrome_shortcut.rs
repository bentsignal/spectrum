use super::*;

pub(super) const WORKBENCH_ACTION_SIZE: Vec2 = Vec2::new(148.0, CONTROL_HEIGHT);

fn shortcut_button_shortcut_rect(rect: Rect) -> Rect {
    let width = if cfg!(target_os = "macos") {
        43.0
    } else {
        51.0
    };
    Rect::from_center_size(
        Pos2::new(rect.right() - 10.0 - width / 2.0, rect.center().y),
        Vec2::new(width, 20.0),
    )
}

pub(super) fn shortcut_action_button(
    ui: &mut egui::Ui,
    size: Vec2,
    label: &str,
    key: &str,
) -> egui::Response {
    let shortcut = if cfg!(target_os = "macos") {
        format!("Command-{key}")
    } else {
        format!("Control-{key}")
    };
    let accessible_label = format!("{label}, {shortcut}");
    let response = ui.add_sized(
        size,
        egui::Button::new(RichText::new(accessible_label).color(Color32::TRANSPARENT)),
    );
    let rect = response.rect;
    ui.painter().text(
        Pos2::new(rect.left() + 10.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(12.0),
        TEXT,
    );
    paint_command_shortcut(ui, shortcut_button_shortcut_rect(rect), key);
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcut_keycap_is_centered_inside_the_complete_control() {
        let control = Rect::from_min_size(Pos2::new(10.0, 20.0), WORKBENCH_ACTION_SIZE);
        let shortcut = shortcut_button_shortcut_rect(control);
        assert_eq!(shortcut.center().y, control.center().y);
        assert_eq!(shortcut.height(), 20.0);
        assert_eq!(control.height(), CONTROL_HEIGHT);
    }

    #[test]
    fn shortcut_action_is_an_accessible_keyboard_button() {
        use egui::accesskit::{Action, Role};

        let context = egui::Context::default();
        context.enable_accesskit();
        let mut semantics = None;
        let _ = context.run_ui(
            egui::RawInput {
                focused: true,
                ..Default::default()
            },
            |ui| {
                let response =
                    shortcut_action_button(ui, WORKBENCH_ACTION_SIZE, "Tools & Actions", "P");
                assert!(response.sense.senses_click());
                assert!(response.sense.is_focusable());
                response.request_focus();
                semantics = context.accesskit_node_builder(response.id, |node| {
                    (
                        node.role(),
                        node.label().map(str::to_owned),
                        node.supports_action(Action::Click),
                        node.supports_action(Action::Focus),
                    )
                });
            },
        );
        assert_eq!(
            semantics,
            Some((
                Role::Button,
                Some(if cfg!(target_os = "macos") {
                    "Tools & Actions, Command-P".to_owned()
                } else {
                    "Tools & Actions, Control-P".to_owned()
                }),
                true,
                true,
            ))
        );

        let mut input = egui::RawInput {
            focused: true,
            ..Default::default()
        };
        input.events.push(egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: Some(egui::Key::Enter),
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        });
        let mut keyboard_clicked = false;
        let _ = context.run_ui(input, |ui| {
            keyboard_clicked =
                shortcut_action_button(ui, WORKBENCH_ACTION_SIZE, "Tools & Actions", "P").clicked();
        });
        assert!(keyboard_clicked);
    }
}
