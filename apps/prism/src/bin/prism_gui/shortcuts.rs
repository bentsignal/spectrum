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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ApplicationShortcutOwner {
    Egui,
    #[cfg(target_os = "macos")]
    NativeMenu,
}

#[cfg(target_os = "macos")]
fn application_shortcut_owner() -> ApplicationShortcutOwner {
    ApplicationShortcutOwner::NativeMenu
}

#[cfg(not(target_os = "macos"))]
fn application_shortcut_owner() -> ApplicationShortcutOwner {
    ApplicationShortcutOwner::Egui
}

fn route_application_shortcut(
    owner: ApplicationShortcutOwner,
    action_available: bool,
    shortcut_pressed: bool,
) -> bool {
    owner == ApplicationShortcutOwner::Egui && action_available && shortcut_pressed
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum GlobalShortcut {
    ToolsAndActions,
    JumpToObject,
    Terminal,
    History,
    UndoRedo,
}

impl GlobalShortcut {
    #[cfg(test)]
    const ALL: [Self; 5] = [
        Self::ToolsAndActions,
        Self::JumpToObject,
        Self::Terminal,
        Self::History,
        Self::UndoRedo,
    ];

    pub(super) fn key(self) -> egui::Key {
        match self {
            Self::ToolsAndActions => egui::Key::P,
            Self::JumpToObject => egui::Key::F,
            Self::Terminal => egui::Key::J,
            Self::History => egui::Key::H,
            Self::UndoRedo => egui::Key::Z,
        }
    }

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::ToolsAndActions => "P",
            Self::JumpToObject => "F",
            Self::Terminal => "J",
            Self::History => "H",
            Self::UndoRedo => "Z",
        }
    }
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

pub(super) fn global_shortcut_pressed(input: &egui::InputState, shortcut: GlobalShortcut) -> bool {
    shortcut_domain(input.modifiers) == Some(ShortcutDomain::Global)
        && input.key_pressed(shortcut.key())
}

fn history_shortcut_pressed(input: &egui::InputState) -> bool {
    #[cfg(target_os = "macos")]
    {
        input.modifiers.command
            && input.modifiers.shift
            && !input.modifiers.alt
            && input.key_pressed(GlobalShortcut::History.key())
    }
    #[cfg(not(target_os = "macos"))]
    {
        global_shortcut_pressed(input, GlobalShortcut::History)
    }
}

fn terminal_shortcut_pressed(input: &egui::InputState) -> bool {
    if !input.key_pressed(GlobalShortcut::Terminal.key()) || input.modifiers.alt {
        return false;
    }
    #[cfg(target_os = "macos")]
    {
        input.modifiers.mac_cmd
    }
    #[cfg(not(target_os = "macos"))]
    {
        // Ctrl-J is a terminal control character. Require Shift for Prism's
        // chrome action so an open terminal can always receive raw Ctrl-J.
        input.modifiers.ctrl && input.modifiers.shift
    }
}

fn rotation_arm_allowed(interaction_active: bool, has_editable_selection: bool) -> bool {
    !interaction_active && has_editable_selection
}

fn reset_tool_after_escape(tool: &mut Tool, status: &mut String, status_error: &mut bool) {
    *tool = Tool::Move;
    *status = Tool::Move.description().into();
    *status_error = false;
}

impl PrismApp {
    pub(super) fn keyboard(&mut self, context: &egui::Context) {
        let application_shortcut_owner = application_shortcut_owner();
        let terminal_pressed = context.input(terminal_shortcut_pressed);
        if route_application_shortcut(
            application_shortcut_owner,
            !self.has_modal_surface(),
            terminal_pressed,
        ) {
            self.toggle_terminal();
            return;
        }
        if self.terminal.visible() {
            return;
        }
        if self.has_modal_surface() || context.egui_wants_keyboard_input() {
            return;
        }
        if route_application_shortcut(
            application_shortcut_owner,
            true,
            context.input(history_shortcut_pressed),
        ) {
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
            } else if input.key_pressed(egui::Key::S) {
                Some(Tool::Shape)
            } else if input.key_pressed(egui::Key::M) {
                Some(Tool::Mask)
            } else if input.key_pressed(egui::Key::W) {
                Some(Tool::Marquee)
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
        if context.input(|input| focused_shortcut_pressed(input, egui::Key::R)) {
            let interaction_active = self.workspace.interaction_active();
            let has_editable_selection = self.selected_layer().is_some_and(|layer| !layer.locked);
            if rotation_arm_allowed(interaction_active, has_editable_selection) {
                self.choose_tool(Tool::Rotate);
            } else if interaction_active {
                self.status = "Finish or cancel the current canvas gesture before rotating".into();
                self.status_error = true;
            } else {
                self.status = "Select an unlocked object to rotate".into();
                self.status_error = true;
            }
        }
        if context.input(|input| global_shortcut_pressed(input, GlobalShortcut::ToolsAndActions)) {
            self.tool_palette = Some(chrome::PaletteState::default());
        }
        if context.input(|input| global_shortcut_pressed(input, GlobalShortcut::JumpToObject)) {
            self.composition_search_focus = true;
        }
        if route_application_shortcut(
            application_shortcut_owner,
            true,
            context.input(|input| global_shortcut_pressed(input, GlobalShortcut::UndoRedo)),
        ) {
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
            if let Some(drag) = self.drag {
                self.workspace.cancel_interaction();
                restore_source_override_after_cancel(
                    &mut self.layer_source_overrides,
                    &self.workspace.document,
                    drag,
                );
            }
            self.tool_palette = None;
            self.shape_palette = None;
            reset_tool_after_escape(&mut self.tool, &mut self.status, &mut self.status_error);
            self.drag = None;
            self.smart_guides = SmartGuides::default();
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
    fn rotate_uses_the_focused_object_shortcut_and_label() {
        let event = egui::Event::Key {
            key: egui::Key::R,
            physical_key: Some(egui::Key::R),
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers {
                alt: true,
                ..Default::default()
            },
        };
        assert!(event_is_focused_shortcut(&event, egui::Key::R));
        assert_eq!(Tool::Rotate.shortcut(), "R");
        assert!(Tool::Rotate.description().starts_with("Rotation armed"));
        assert!(rotation_arm_allowed(false, true));
        assert!(!rotation_arm_allowed(false, false));
        assert!(!rotation_arm_allowed(true, true));
    }

    #[test]
    fn escape_resets_an_armed_rotation_status_to_move() {
        let mut tool = Tool::Rotate;
        let mut status = Tool::Rotate.description().to_owned();
        let mut status_error = true;
        reset_tool_after_escape(&mut tool, &mut status, &mut status_error);
        assert_eq!(tool, Tool::Move);
        assert_eq!(status, Tool::Move.description());
        assert!(!status_error);
    }

    #[test]
    fn platform_history_chord_belongs_to_the_global_history_surface() {
        let mut input = egui::RawInput::default();
        input.modifiers.command = true;
        #[cfg(target_os = "macos")]
        {
            input.modifiers.shift = true;
        }
        input.events.push(egui::Event::Key {
            key: egui::Key::H,
            physical_key: Some(egui::Key::H),
            pressed: true,
            repeat: false,
            modifiers: input.modifiers,
        });
        let context = egui::Context::default();
        context.begin_pass(input);
        assert!(context.input(history_shortcut_pressed));
        let _ = context.end_pass();
    }

    #[test]
    fn global_shortcuts_match_labels_without_collisions() {
        assert_eq!(GlobalShortcut::ToolsAndActions.key(), egui::Key::P);
        assert_eq!(GlobalShortcut::ToolsAndActions.label(), "P");
        assert_eq!(GlobalShortcut::JumpToObject.key(), egui::Key::F);
        assert_eq!(GlobalShortcut::JumpToObject.label(), "F");
        assert_eq!(GlobalShortcut::Terminal.key(), egui::Key::J);
        assert_eq!(GlobalShortcut::Terminal.label(), "J");
        for (index, shortcut) in GlobalShortcut::ALL.iter().enumerate() {
            for other in &GlobalShortcut::ALL[index + 1..] {
                assert_ne!(shortcut.key(), other.key());
            }
        }
    }

    #[test]
    fn application_shortcuts_have_one_owner_and_respect_availability() {
        #[cfg(target_os = "macos")]
        assert_eq!(
            application_shortcut_owner(),
            ApplicationShortcutOwner::NativeMenu
        );
        #[cfg(not(target_os = "macos"))]
        assert_eq!(application_shortcut_owner(), ApplicationShortcutOwner::Egui);
        assert!(route_application_shortcut(
            ApplicationShortcutOwner::Egui,
            true,
            true
        ));
        #[cfg(target_os = "macos")]
        assert!(!route_application_shortcut(
            ApplicationShortcutOwner::NativeMenu,
            true,
            true
        ));
        assert!(!route_application_shortcut(
            ApplicationShortcutOwner::Egui,
            false,
            true
        ));
        assert!(!route_application_shortcut(
            ApplicationShortcutOwner::Egui,
            true,
            false
        ));
    }

    #[test]
    fn terminal_toggle_uses_a_platform_chord_that_preserves_control_j() {
        let mut input = egui::RawInput::default();
        #[cfg(target_os = "macos")]
        {
            input.modifiers.mac_cmd = true;
            input.modifiers.command = true;
        }
        #[cfg(not(target_os = "macos"))]
        {
            input.modifiers.ctrl = true;
            input.modifiers.command = true;
            input.modifiers.shift = true;
        }
        input.events.push(egui::Event::Key {
            key: egui::Key::J,
            physical_key: Some(egui::Key::J),
            pressed: true,
            repeat: false,
            modifiers: input.modifiers,
        });
        let context = egui::Context::default();
        context.begin_pass(input);
        assert!(context.input(terminal_shortcut_pressed));
        let _ = context.end_pass();
    }
}
