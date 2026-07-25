use prism_core::PrismLiveInteractionState;

use super::*;

impl PrismApp {
    pub(super) fn live_interaction_state(&self) -> PrismLiveInteractionState {
        let active = self.workspace.interaction_active()
            || self.drag.is_some()
            || self.inline_text_editor.is_some()
            || self.pen.live_interaction_active()
            || self.brush.live_interaction_active()
            || !self.selection_ui.lasso_points.is_empty()
            || self.layer_drag.is_some()
            || self.rename_layer.is_some()
            || self.rename_document.is_some()
            || self.new_dialog.is_some()
            || self.delete_confirmation.is_some()
            || self.move_project_dialog.is_some();
        if active {
            PrismLiveInteractionState::Active
        } else {
            PrismLiveInteractionState::Idle
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neutral_interaction_state_values_remain_distinct() {
        assert_ne!(
            PrismLiveInteractionState::Idle,
            PrismLiveInteractionState::Active
        );
    }
}
