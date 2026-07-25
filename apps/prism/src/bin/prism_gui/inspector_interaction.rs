use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum InspectorWidgetFocusLossAction {
    None,
    Commit,
    Cancel,
}

pub(super) fn focused_workspace_interaction_escape_pressed(
    context: &egui::Context,
    owned_by_inline_text: bool,
    owned_by_brush_color_picker: bool,
) -> bool {
    context.egui_wants_keyboard_input()
        && !owned_by_inline_text
        && !owned_by_brush_color_picker
        && context.input(|input| input.key_pressed(egui::Key::Escape))
}

pub(super) fn cancel_workspace_interaction_and_arm_frame(
    workspace: &mut Workspace,
    suppress_inspector_widget_commands: &mut bool,
) -> bool {
    if !workspace.interaction_active() || !workspace.cancel_interaction() {
        return false;
    }
    *suppress_inspector_widget_commands = true;
    true
}

impl PrismApp {
    pub(super) fn cancel_workspace_interaction_for_escape_frame(&mut self) -> bool {
        if !cancel_workspace_interaction_and_arm_frame(
            &mut self.workspace,
            &mut self.suppress_inspector_widget_commands,
        ) {
            return false;
        }
        self.sync_active_raster_sources();
        self.layer_visual_dirty
            .extend(self.workspace.document.layers.iter().map(|layer| layer.id));
        true
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct InspectorWidgetFocusLossEvidence {
    lost_focus: bool,
    enter_or_tab_pressed: bool,
    pointer_pressed: bool,
    focus_transferred: bool,
}

fn focus_loss_from_evidence(
    evidence: InspectorWidgetFocusLossEvidence,
) -> InspectorWidgetFocusLossAction {
    if !evidence.lost_focus {
        InspectorWidgetFocusLossAction::None
    } else if evidence.enter_or_tab_pressed
        || evidence.pointer_pressed
        || evidence.focus_transferred
    {
        InspectorWidgetFocusLossAction::Commit
    } else {
        InspectorWidgetFocusLossAction::Cancel
    }
}

pub(super) fn inspector_widget_focus_loss_action(
    response: &egui::Response,
) -> InspectorWidgetFocusLossAction {
    let (enter_or_tab_pressed, pointer_pressed) = response.ctx.input(|input| {
        (
            input.events.iter().any(|event| {
                matches!(
                    event,
                    egui::Event::Key {
                        key: egui::Key::Enter | egui::Key::Tab,
                        pressed: true,
                        repeat: false,
                        ..
                    }
                )
            }),
            input
                .events
                .iter()
                .any(|event| matches!(event, egui::Event::PointerButton { pressed: true, .. })),
        )
    });
    focus_loss_from_evidence(InspectorWidgetFocusLossEvidence {
        lost_focus: response.lost_focus(),
        enter_or_tab_pressed,
        pointer_pressed,
        focus_transferred: response.ctx.memory(|memory| memory.focused().is_some()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn classify_real_focus_loss(
        event: Option<egui::Event>,
        transfer_focus: bool,
    ) -> InspectorWidgetFocusLossAction {
        let context = egui::Context::default();
        let mut text = "0.50".to_owned();
        context.begin_pass(egui::RawInput::default());
        let first = egui::Area::new(egui::Id::new("focus-loss-evidence"))
            .show(&context, |ui| ui.text_edit_singleline(&mut text))
            .inner;
        first.request_focus();
        let _ = context.end_pass();
        context.memory_mut(|memory| memory.request_focus(first.id));
        context.begin_pass(egui::RawInput::default());
        let response = egui::Area::new(egui::Id::new("focus-loss-evidence"))
            .show(&context, |ui| ui.text_edit_singleline(&mut text))
            .inner;
        let _ = context.end_pass();
        if transfer_focus {
            context.memory_mut(|memory| memory.request_focus(egui::Id::new("next-widget")));
        } else {
            context.memory_mut(|memory| memory.surrender_focus(response.id));
        }
        let mut input = egui::RawInput::default();
        input.events.extend(event);
        context.begin_pass(input);
        assert!(response.lost_focus());
        let action = inspector_widget_focus_loss_action(&response);
        let _ = context.end_pass();
        action
    }

    #[test]
    fn ambiguity_cancels_but_explicit_focus_loss_commits() {
        let ambiguous = InspectorWidgetFocusLossEvidence {
            lost_focus: true,
            ..Default::default()
        };
        assert_eq!(
            focus_loss_from_evidence(ambiguous),
            InspectorWidgetFocusLossAction::Cancel
        );
        for evidence in [
            InspectorWidgetFocusLossEvidence {
                enter_or_tab_pressed: true,
                ..ambiguous
            },
            InspectorWidgetFocusLossEvidence {
                pointer_pressed: true,
                ..ambiguous
            },
            InspectorWidgetFocusLossEvidence {
                focus_transferred: true,
                ..ambiguous
            },
        ] {
            assert_eq!(
                focus_loss_from_evidence(evidence),
                InspectorWidgetFocusLossAction::Commit
            );
        }
        assert_eq!(
            focus_loss_from_evidence(Default::default()),
            InspectorWidgetFocusLossAction::None
        );
    }

    #[test]
    fn real_responses_distinguish_native_ambiguity_from_commit_evidence() {
        assert_eq!(
            classify_real_focus_loss(None, false),
            InspectorWidgetFocusLossAction::Cancel
        );
        for event in [
            egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: Some(egui::Key::Enter),
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            },
            egui::Event::Key {
                key: egui::Key::Tab,
                physical_key: Some(egui::Key::Tab),
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            },
            egui::Event::PointerButton {
                pos: egui::Pos2::new(20.0, 20.0),
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ] {
            assert_eq!(
                classify_real_focus_loss(Some(event), false),
                InspectorWidgetFocusLossAction::Commit
            );
        }
        assert_eq!(
            classify_real_focus_loss(None, true),
            InspectorWidgetFocusLossAction::Commit
        );
    }
}
