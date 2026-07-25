use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ShortcutLabels {
    pub(super) history: &'static str,
    pub(super) terminal: &'static str,
    pub(super) undo: &'static str,
    pub(super) redo: &'static str,
    pub(super) new_document: &'static str,
    pub(super) close_document: &'static str,
    pub(super) commit_text: &'static str,
    pub(super) terminal_meta: &'static str,
    pub(super) terminal_clipboard: &'static str,
}

impl ShortcutLabels {
    const fn for_platform(macos: bool) -> Self {
        if macos {
            Self {
                history: "⌘H",
                terminal: "⌘J",
                undo: "⌘Z",
                redo: "⇧⌘Z",
                new_document: "⌘N",
                close_document: "⌘W",
                commit_text: "⌘↵",
                terminal_meta: "⌥ key sends Meta",
                terminal_clipboard: "⌘C copy · ⌘V paste",
            }
        } else {
            Self {
                history: "Ctrl+H",
                terminal: "Ctrl+Shift+J",
                undo: "Ctrl+Z",
                redo: "Ctrl+Shift+Z",
                new_document: "Ctrl+N",
                close_document: "Ctrl+W",
                commit_text: "Ctrl+Enter",
                terminal_meta: "Alt+key sends Meta",
                terminal_clipboard: "Ctrl+Shift+C copy · Ctrl+Shift+V paste",
            }
        }
    }
}

pub(super) const SHORTCUT_LABELS: ShortcutLabels =
    ShortcutLabels::for_platform(cfg!(target_os = "macos"));

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
enum DeleteShortcutAction {
    DeleteSelectedPixels(u64),
    ConfirmLayerDeletion(Option<u64>),
    MissingRasterTarget,
}

fn delete_shortcut_action(
    pixel_selection_active: bool,
    selected_layer: Option<u64>,
) -> DeleteShortcutAction {
    match (pixel_selection_active, selected_layer) {
        (true, Some(id)) => DeleteShortcutAction::DeleteSelectedPixels(id),
        (true, None) => DeleteShortcutAction::MissingRasterTarget,
        (false, selected) => DeleteShortcutAction::ConfirmLayerDeletion(selected),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum GlobalShortcut {
    ToolsAndActions,
    JumpToObject,
    Terminal,
    History,
    UndoRedo,
    SelectAll,
    Deselect,
}

impl GlobalShortcut {
    #[cfg(test)]
    const ALL: [Self; 7] = [
        Self::ToolsAndActions,
        Self::JumpToObject,
        Self::Terminal,
        Self::History,
        Self::UndoRedo,
        Self::SelectAll,
        Self::Deselect,
    ];

    pub(super) fn key(self) -> egui::Key {
        match self {
            Self::ToolsAndActions => egui::Key::P,
            Self::JumpToObject => egui::Key::F,
            Self::Terminal => egui::Key::J,
            Self::History => egui::Key::H,
            Self::UndoRedo => egui::Key::Z,
            Self::SelectAll => egui::Key::A,
            Self::Deselect => egui::Key::D,
        }
    }

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::ToolsAndActions => "P",
            Self::JumpToObject => "F",
            Self::Terminal => "J",
            Self::History => "H",
            Self::UndoRedo => "Z",
            Self::SelectAll => "A",
            Self::Deselect => "D",
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
    global_shortcut_pressed(input, GlobalShortcut::History)
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

fn close_document_shortcut_pressed(input: &egui::InputState) -> bool {
    shortcut_domain(input.modifiers) == Some(ShortcutDomain::Global)
        && input.key_pressed(egui::Key::W)
}

fn rotation_arm_allowed(interaction_active: bool, has_editable_selection: bool) -> bool {
    !interaction_active && has_editable_selection
}

pub(super) fn canvas_interaction_active(
    canvas_drag_active: bool,
    workspace_interaction_active: bool,
) -> bool {
    canvas_drag_active || workspace_interaction_active
}

fn reset_tool_after_escape(tool: &mut Tool, status: &mut String, status_error: &mut bool) {
    *tool = Tool::Move;
    *status = Tool::Move.description().into();
    *status_error = false;
}

fn toolbar_gradient_editor_owns_escape(editor_open: bool, escape_pressed: bool) -> bool {
    editor_open && escape_pressed
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
        if route_application_shortcut(
            application_shortcut_owner,
            self.workspace_initialized && !self.has_modal_surface(),
            context.input(close_document_shortcut_pressed),
        ) {
            self.close_active_tab();
            return;
        }
        if self.terminal.visible() {
            return;
        }
        if self.has_modal_surface() {
            return;
        }
        if toolbar_gradient_editor_owns_escape(
            self.toolbar_prototype.gradient_editor_open(),
            context.input(|input| input.key_pressed(egui::Key::Escape)),
        ) {
            // Leave Escape unconsumed for the nested picker or outer editor.
            return;
        }
        let inline_text_owns_escape = self.inline_text_editor.is_some();
        let brush_color_picker_owns_escape = self.brush_color_picker_open();
        if focused_workspace_interaction_escape_pressed(
            context,
            inline_text_owns_escape,
            brush_color_picker_owns_escape,
        ) && self.cancel_workspace_interaction_for_escape_frame()
        {
            return;
        }
        if context.egui_wants_keyboard_input() {
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
        if self.tool == Tool::Pen && context.input(|input| input.key_pressed(egui::Key::Enter)) {
            self.finish_pen_path(false);
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
            } else if input.key_pressed(egui::Key::P) {
                Some(Tool::Pen)
            } else if input.key_pressed(egui::Key::B) {
                Some(Tool::Brush)
            } else if input.key_pressed(egui::Key::E) {
                Some(Tool::Eraser)
            } else if input.key_pressed(egui::Key::K) {
                Some(Tool::Mask)
            } else if input.key_pressed(egui::Key::M) {
                Some(Tool::Marquee)
            } else if input.key_pressed(egui::Key::L) {
                Some(Tool::Lasso)
            } else if input.key_pressed(egui::Key::W) {
                Some(Tool::MagicWand)
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
        let selection_actions_available = !self.history.visible
            && !canvas_interaction_active(self.drag.is_some(), self.workspace.interaction_active());
        if route_application_shortcut(
            application_shortcut_owner,
            selection_actions_available,
            context.input(|input| global_shortcut_pressed(input, GlobalShortcut::SelectAll)),
        ) {
            self.select_all_pixels();
            return;
        }
        if route_application_shortcut(
            application_shortcut_owner,
            selection_actions_available && self.workspace.document.selection.is_some(),
            context.input(|input| global_shortcut_pressed(input, GlobalShortcut::Deselect)),
        ) {
            self.deselect_pixels();
            return;
        }
        if context.input(|input| {
            input.key_pressed(egui::Key::Delete) || input.key_pressed(egui::Key::Backspace)
        }) {
            match delete_shortcut_action(
                self.workspace.document.selection.is_some(),
                self.workspace.document.selected,
            ) {
                DeleteShortcutAction::DeleteSelectedPixels(id) => {
                    self.execute(Command::DeleteSelectedPixels { id });
                }
                DeleteShortcutAction::MissingRasterTarget => {
                    self.status =
                        "Select a raster image layer before deleting selected pixels".into();
                    self.status_error = true;
                }
                DeleteShortcutAction::ConfirmLayerDeletion(selected) => {
                    self.delete_confirmation = selected;
                }
            }
        }
        if self.brush_color_picker_open()
            && context
                .input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            self.cancel_brush_color_picker(context);
            return;
        }
        if context.input(|input| input.key_pressed(egui::Key::Escape)) {
            self.cancel_pen();
            self.cancel_brush();
            self.cancel_lasso();
            self.cancel_workspace_interaction_for_escape_frame();
            if let Some(drag) = self.drag {
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
    fn shortcut_labels_match_each_platform_chord() {
        let macos = ShortcutLabels::for_platform(true);
        assert_eq!(macos.history, "⌘H");
        assert_eq!(macos.terminal, "⌘J");
        assert_eq!(macos.redo, "⇧⌘Z");
        assert_eq!(macos.new_document, "⌘N");
        assert_eq!(macos.close_document, "⌘W");
        assert_eq!(macos.terminal_meta, "⌥ key sends Meta");
        assert_eq!(macos.terminal_clipboard, "⌘C copy · ⌘V paste");

        let portable = ShortcutLabels::for_platform(false);
        assert_eq!(portable.history, "Ctrl+H");
        assert_eq!(portable.terminal, "Ctrl+Shift+J");
        assert_eq!(portable.undo, "Ctrl+Z");
        assert_eq!(portable.redo, "Ctrl+Shift+Z");
        assert_eq!(portable.new_document, "Ctrl+N");
        assert_eq!(portable.close_document, "Ctrl+W");
        assert_eq!(portable.commit_text, "Ctrl+Enter");
        assert_eq!(portable.terminal_meta, "Alt+key sends Meta");
        assert_eq!(
            portable.terminal_clipboard,
            "Ctrl+Shift+C copy · Ctrl+Shift+V paste"
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
    fn toolbar_gradient_editor_routes_escape_before_the_global_tool_reset() {
        let mut tool = Tool::Shape;
        let mut status = Tool::Shape.description().to_owned();
        let mut status_error = false;

        if !toolbar_gradient_editor_owns_escape(true, true) {
            reset_tool_after_escape(&mut tool, &mut status, &mut status_error);
        }

        assert_eq!(tool, Tool::Shape);
        assert_eq!(status, Tool::Shape.description());
        assert!(!toolbar_gradient_editor_owns_escape(false, true));
        assert!(!toolbar_gradient_editor_owns_escape(true, false));
    }

    #[test]
    fn platform_history_chord_belongs_to_the_global_history_surface() {
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
        assert_eq!(GlobalShortcut::SelectAll.key(), egui::Key::A);
        assert_eq!(GlobalShortcut::Deselect.key(), egui::Key::D);
        for (index, shortcut) in GlobalShortcut::ALL.iter().enumerate() {
            for other in &GlobalShortcut::ALL[index + 1..] {
                assert_ne!(shortcut.key(), other.key());
            }
        }
    }

    #[test]
    fn selection_chords_use_the_portable_global_shortcut_domain() {
        for shortcut in [GlobalShortcut::SelectAll, GlobalShortcut::Deselect] {
            let mut input = egui::RawInput::default();
            input.modifiers.command = true;
            input.events.push(egui::Event::Key {
                key: shortcut.key(),
                physical_key: Some(shortcut.key()),
                pressed: true,
                repeat: false,
                modifiers: input.modifiers,
            });
            let context = egui::Context::default();
            context.begin_pass(input);
            assert!(context.input(|input| global_shortcut_pressed(input, shortcut)));
            let _ = context.end_pass();
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
    fn delete_and_backspace_prioritize_pixel_selection_over_layer_deletion() {
        assert_eq!(
            delete_shortcut_action(true, Some(41)),
            DeleteShortcutAction::DeleteSelectedPixels(41)
        );
        assert_eq!(
            delete_shortcut_action(true, None),
            DeleteShortcutAction::MissingRasterTarget
        );
        assert_eq!(
            delete_shortcut_action(false, Some(41)),
            DeleteShortcutAction::ConfirmLayerDeletion(Some(41))
        );
        assert_eq!(
            delete_shortcut_action(false, None),
            DeleteShortcutAction::ConfirmLayerDeletion(None)
        );
    }

    #[test]
    fn close_document_uses_the_global_command_or_control_w_domain() {
        let mut input = egui::RawInput::default();
        input.modifiers.command = true;
        input.events.push(egui::Event::Key {
            key: egui::Key::W,
            physical_key: Some(egui::Key::W),
            pressed: true,
            repeat: false,
            modifiers: input.modifiers,
        });
        let context = egui::Context::default();
        context.begin_pass(input);
        assert!(context.input(close_document_shortcut_pressed));
        let _ = context.end_pass();
    }

    #[test]
    fn draw_drag_owns_selection_shortcuts_without_a_workspace_interaction() {
        let interaction_active = canvas_interaction_active(true, false);
        assert!(interaction_active);
        assert!(!route_application_shortcut(
            ApplicationShortcutOwner::Egui,
            !interaction_active,
            true
        ));
        assert!(canvas_interaction_active(false, true));
        assert!(!canvas_interaction_active(false, false));
    }

    #[test]
    fn escape_cancels_an_inspector_preview_without_bytes_or_revision() {
        let project = std::env::temp_dir().join(format!(
            "prism-inspector-escape-{}.prism",
            spectrum_revisions::SessionId::new()
        ));
        let actor = spectrum_revisions::Actor {
            id: "human:inspector-escape".into(),
            display_name: "Inspector escape".into(),
            kind: spectrum_revisions::ActorKind::Human,
        };
        let session = spectrum_revisions::SessionId::new();
        let mut workspace = Workspace::create_durable(
            Document::new("Inspector Escape", 64, 48),
            &project,
            actor,
            session,
        )
        .unwrap();
        workspace
            .execute(Command::AddRectangle {
                name: None,
                width: 32,
                height: 24,
                color: [255; 4],
                corner_radius: 0.0,
                x: 0.0,
                y: 0.0,
            })
            .unwrap();
        workspace.save(None).unwrap();
        let bytes_before = std::fs::read(&project).unwrap();
        let revisions_before = workspace.history().unwrap().unwrap().revisions.len();
        let document_before = workspace.document.clone();

        workspace.begin_interaction().unwrap();
        workspace
            .preview(Command::SetShapeFill {
                id: 1,
                fill: Some(prism_core::ShapeFill::Gradient(prism_core::ShapeGradient {
                    kind: prism_core::GradientKind::Radial,
                    ..Default::default()
                })),
            })
            .unwrap();
        assert!(workspace.interaction_active());
        let context = egui::Context::default();
        let mut edited_value = "0.50".to_owned();
        context.begin_pass(egui::RawInput::default());
        let focused_response = egui::Area::new(egui::Id::new("unfocused-inspector-test"))
            .show(&context, |ui| ui.text_edit_singleline(&mut edited_value))
            .inner;
        focused_response.request_focus();
        let _ = context.end_pass();
        context.memory_mut(|memory| memory.request_focus(focused_response.id));
        context.begin_pass(egui::RawInput::default());
        let mut stale_widget_response = egui::Area::new(egui::Id::new("unfocused-inspector-test"))
            .show(&context, |ui| ui.text_edit_singleline(&mut edited_value))
            .inner;
        assert!(context.egui_wants_keyboard_input());
        let _ = context.end_pass();
        context.memory_mut(|memory| memory.surrender_focus(focused_response.id));
        let mut input = egui::RawInput::default();
        input.events.push(egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: Some(egui::Key::Escape),
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        });
        context.begin_pass(input);
        stale_widget_response.mark_changed();
        assert!(!context.egui_wants_keyboard_input());
        assert!(context.input(|input| input.key_pressed(egui::Key::Escape)));
        assert!(stale_widget_response.changed());
        assert!(stale_widget_response.lost_focus());
        assert_eq!(
            inspector_widget_focus_loss_action(&stale_widget_response),
            InspectorWidgetFocusLossAction::Cancel
        );
        let mut suppress_for_frame = false;
        assert!(cancel_workspace_interaction_and_arm_frame(
            &mut workspace,
            &mut suppress_for_frame
        ));
        assert!(suppress_for_frame);
        let suppressed_phases =
            inspector_widget_command_phases(&stale_widget_response, suppress_for_frame);
        assert_eq!(suppressed_phases, InspectorWidgetCommandPhases::default());
        if suppressed_phases.apply {
            workspace
                .preview(Command::SetShapeFill {
                    id: 1,
                    fill: Some(prism_core::ShapeFill::Gradient(prism_core::ShapeGradient {
                        offset: 0.5,
                        ..Default::default()
                    })),
                })
                .unwrap();
        }
        if suppressed_phases.finish {
            workspace.commit_interaction().unwrap();
        }
        suppress_for_frame = false;
        assert_eq!(
            inspector_widget_command_phases(&stale_widget_response, suppress_for_frame),
            InspectorWidgetCommandPhases {
                begin: false,
                apply: true,
                finish: true,
            }
        );
        let _ = context.end_pass();
        assert!(!workspace.interaction_active());
        assert_eq!(workspace.document, document_before);
        assert_eq!(
            workspace.history().unwrap().unwrap().revisions.len(),
            revisions_before
        );
        assert_eq!(std::fs::read(&project).unwrap(), bytes_before);
        drop(workspace);
        std::fs::remove_file(project).unwrap();
    }

    #[test]
    fn focused_egui_escape_routes_to_an_active_inspector_preview() {
        let project = std::env::temp_dir().join(format!(
            "prism-focused-inspector-escape-{}.prism",
            spectrum_revisions::SessionId::new()
        ));
        let actor = spectrum_revisions::Actor {
            id: "human:focused-inspector-escape".into(),
            display_name: "Focused inspector escape".into(),
            kind: spectrum_revisions::ActorKind::Human,
        };
        let session = spectrum_revisions::SessionId::new();
        let mut workspace = Workspace::create_durable(
            Document::new("Focused Inspector Escape", 64, 48),
            &project,
            actor,
            session,
        )
        .unwrap();
        workspace
            .execute(Command::AddRectangle {
                name: None,
                width: 32,
                height: 24,
                color: [255; 4],
                corner_radius: 0.0,
                x: 0.0,
                y: 0.0,
            })
            .unwrap();
        workspace.save(None).unwrap();
        let bytes_before = std::fs::read(&project).unwrap();
        let revisions_before = workspace.history().unwrap().unwrap().revisions.len();
        let document_before = workspace.document.clone();

        workspace.begin_interaction().unwrap();
        workspace
            .preview(Command::SetShapeFill {
                id: 1,
                fill: Some(prism_core::ShapeFill::Gradient(prism_core::ShapeGradient {
                    offset: 0.5,
                    ..Default::default()
                })),
            })
            .unwrap();

        let context = egui::Context::default();
        let mut focused_value = "0.50".to_owned();
        context.begin_pass(egui::RawInput::default());
        let focused = egui::Area::new(egui::Id::new("focused-inspector-test"))
            .show(&context, |ui| ui.text_edit_singleline(&mut focused_value))
            .inner;
        focused.request_focus();
        let _ = context.end_pass();
        context.memory_mut(|memory| memory.request_focus(focused.id));

        let mut input = egui::RawInput::default();
        input.events.push(egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: Some(egui::Key::Escape),
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        });
        context.begin_pass(input);
        context.memory_mut(|memory| memory.request_focus(focused.id));
        assert!(context.egui_wants_keyboard_input());
        assert!(focused_workspace_interaction_escape_pressed(
            &context, false, false
        ));
        let mut suppress_for_frame = false;
        assert!(cancel_workspace_interaction_and_arm_frame(
            &mut workspace,
            &mut suppress_for_frame
        ));
        assert!(suppress_for_frame);
        let mut stale_widget_response = focused.clone();
        stale_widget_response.mark_changed();
        stale_widget_response.surrender_focus();
        assert!(stale_widget_response.changed());
        assert!(stale_widget_response.lost_focus());
        assert_eq!(
            inspector_widget_focus_loss_action(&stale_widget_response),
            InspectorWidgetFocusLossAction::Cancel
        );
        let suppressed_phases =
            inspector_widget_command_phases(&stale_widget_response, suppress_for_frame);
        assert_eq!(suppressed_phases, InspectorWidgetCommandPhases::default());
        if suppressed_phases.apply {
            workspace
                .preview(Command::SetShapeFill {
                    id: 1,
                    fill: Some(prism_core::ShapeFill::Gradient(prism_core::ShapeGradient {
                        offset: 0.5,
                        ..Default::default()
                    })),
                })
                .unwrap();
        }
        if suppressed_phases.finish {
            workspace.commit_interaction().unwrap();
        }
        suppress_for_frame = false;
        assert_eq!(
            inspector_widget_command_phases(&stale_widget_response, suppress_for_frame),
            InspectorWidgetCommandPhases {
                begin: false,
                apply: true,
                finish: true,
            },
            "the suppression must apply only to the Escape frame"
        );
        let _ = context.end_pass();

        assert!(!workspace.interaction_active());
        assert_eq!(workspace.document, document_before);
        assert_eq!(
            workspace.history().unwrap().unwrap().revisions.len(),
            revisions_before
        );
        assert_eq!(std::fs::read(&project).unwrap(), bytes_before);
        for (inline_text_owns_escape, brush_color_picker_owns_escape) in
            [(true, false), (false, true)]
        {
            workspace.begin_interaction().unwrap();
            workspace
                .preview(Command::SetShapeFill {
                    id: 1,
                    fill: Some(prism_core::ShapeFill::Gradient(prism_core::ShapeGradient {
                        offset: 0.25,
                        ..Default::default()
                    })),
                })
                .unwrap();
            assert!(!focused_workspace_interaction_escape_pressed(
                &context,
                inline_text_owns_escape,
                brush_color_picker_owns_escape
            ));
            assert!(workspace.interaction_active());
            assert!(workspace.cancel_interaction());
        }
        assert_eq!(workspace.document, document_before);
        assert_eq!(
            workspace.history().unwrap().unwrap().revisions.len(),
            revisions_before
        );
        assert_eq!(std::fs::read(&project).unwrap(), bytes_before);
        drop(workspace);
        std::fs::remove_file(project).unwrap();
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
