use super::*;

/// Prism assigns modifiers by interaction domain rather than feature:
///
/// - bare letters arm canvas tools;
/// - Command on macOS or Ctrl on Windows/Linux opens global surfaces/actions;
/// - Option on macOS or Alt on Windows/Linux operates on the focused object.
///
/// Keep new shortcuts inside one of these domains instead of assigning an
/// available chord ad hoc.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ShortcutDomain {
    Tool,
    Global,
    FocusedObject,
}

pub(super) fn shortcut_domain(modifiers: egui::Modifiers) -> Option<ShortcutDomain> {
    if modifiers.command && !modifiers.alt {
        Some(ShortcutDomain::Global)
    } else if modifiers.alt && !modifiers.command && !modifiers.ctrl && !modifiers.shift {
        Some(ShortcutDomain::FocusedObject)
    } else if !modifiers.alt
        && !modifiers.command
        && !modifiers.ctrl
        && !modifiers.mac_cmd
        && !modifiers.shift
    {
        Some(ShortcutDomain::Tool)
    } else {
        None
    }
}

fn event_is_focused_shortcut(event: &egui::Event, key: egui::Key) -> bool {
    matches!(
        event,
        egui::Event::Key {
            physical_key: Some(physical_key),
            pressed: true,
            repeat: false,
            modifiers,
            ..
        } if *physical_key == key
            && shortcut_domain(*modifiers) == Some(ShortcutDomain::FocusedObject)
    )
}

pub(super) fn focused_shortcut_pressed(input: &egui::InputState, key: egui::Key) -> bool {
    shortcut_domain(input.modifiers) == Some(ShortcutDomain::FocusedObject)
        && (input.key_pressed(key)
            || input
                .events
                .iter()
                .any(|event| event_is_focused_shortcut(event, key)))
}

pub(super) fn global_shortcut_pressed(input: &egui::InputState, key: egui::Key) -> bool {
    shortcut_domain(input.modifiers) == Some(ShortcutDomain::Global) && input.key_pressed(key)
}

impl PrismApp {
    pub(super) fn keyboard(&mut self, context: &egui::Context) {
        if context.egui_wants_keyboard_input() {
            return;
        }
        if context.input(|input| global_shortcut_pressed(input, egui::Key::H)) {
            self.toggle_history();
            return;
        }
        let chosen_tool = context.input(|input| {
            if shortcut_domain(input.modifiers) != Some(ShortcutDomain::Tool) {
                return None;
            }
            if input.key_pressed(egui::Key::V) {
                Some(Tool::Move)
            } else if input.key_pressed(egui::Key::C) {
                Some(Tool::Crop)
            } else if input.key_pressed(egui::Key::T) {
                Some(Tool::Text)
            } else if input.key_pressed(egui::Key::R) {
                Some(Tool::Rectangle)
            } else if input.key_pressed(egui::Key::U) {
                Some(Tool::Ellipse)
            } else if input.key_pressed(egui::Key::M) {
                Some(Tool::Mask)
            } else {
                None
            }
        });
        if let Some(tool) = chosen_tool {
            self.choose_tool(tool);
        }
        if context.input(|input| focused_shortcut_pressed(input, egui::Key::E)) {
            self.edit_focused();
        }
        if context.input(|input| focused_shortcut_pressed(input, egui::Key::D))
            && let Some(id) = self.workspace.document.selected
        {
            self.execute(Command::DuplicateLayer { id });
        }
        if context.input(|input| global_shortcut_pressed(input, egui::Key::K)) {
            self.tool_palette = Some(String::new());
        }
        if context.input(|input| global_shortcut_pressed(input, egui::Key::J)) {
            self.composition_search_focus = true;
        }
        if context.input(|input| global_shortcut_pressed(input, egui::Key::Z)) {
            if context.input(|input| input.modifiers.shift) {
                self.execute(Command::Redo);
            } else {
                self.execute(Command::Undo);
            }
        }
        if context.input(|input| {
            input.key_pressed(egui::Key::Delete) || input.key_pressed(egui::Key::Backspace)
        }) {
            self.delete_confirmation = self.workspace.document.selected;
        }
        if context.input(|input| input.key_pressed(egui::Key::Escape)) {
            self.tool_palette = None;
            self.tool = Tool::Move;
            self.drag = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modifiers_select_one_intentional_domain() {
        assert_eq!(
            shortcut_domain(egui::Modifiers::default()),
            Some(ShortcutDomain::Tool)
        );
        assert_eq!(
            shortcut_domain(egui::Modifiers {
                command: true,
                ..Default::default()
            }),
            Some(ShortcutDomain::Global)
        );
        assert_eq!(
            shortcut_domain(egui::Modifiers {
                alt: true,
                ..Default::default()
            }),
            Some(ShortcutDomain::FocusedObject)
        );
        assert_eq!(
            shortcut_domain(egui::Modifiers {
                alt: true,
                command: true,
                ..Default::default()
            }),
            None
        );
    }

    #[test]
    fn focused_shortcuts_accept_a_physical_key_when_option_changes_the_character() {
        let event = egui::Event::Key {
            key: egui::Key::E,
            physical_key: Some(egui::Key::E),
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers {
                alt: true,
                ..Default::default()
            },
        };
        assert!(event_is_focused_shortcut(&event, egui::Key::E));
    }

    #[test]
    fn command_h_belongs_to_the_global_history_surface() {
        let mut input = egui::RawInput::default();
        input.modifiers.command = true;
        input.events.push(egui::Event::Key {
            key: egui::Key::H,
            physical_key: Some(egui::Key::H),
            pressed: true,
            repeat: false,
            modifiers: input.modifiers,
        });
        let context = egui::Context::default();
        context.begin_pass(input);
        assert!(context.input(|state| global_shortcut_pressed(state, egui::Key::H)));
        let _ = context.end_pass();
    }
}
