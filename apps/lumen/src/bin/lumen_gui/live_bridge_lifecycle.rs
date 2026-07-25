use lumen_core::{LumenLiveDrainReport, LumenLiveInteractionState};

use super::*;

const CATALOG_BINDING_KEY: u64 = 0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LiveRefreshKind {
    Applied,
    OutcomeUnknown,
    ReopenRequired,
}

fn live_refresh_kind(report: LumenLiveDrainReport) -> Option<LiveRefreshKind> {
    if report.reopen_required {
        Some(LiveRefreshKind::ReopenRequired)
    } else if report.workspace_changed && report.outcome_unknown {
        Some(LiveRefreshKind::OutcomeUnknown)
    } else if report.applied > 0 || report.workspace_changed {
        Some(LiveRefreshKind::Applied)
    } else {
        None
    }
}

impl LumenApp {
    pub(super) fn register_live_catalog(&mut self) -> Result<(), String> {
        let Some(registry) = &mut self.live_bridge else {
            return Err("Lumen live bridge is unavailable".into());
        };
        match registry.register(CATALOG_BINDING_KEY, &self.workspace) {
            Ok(registration) => {
                let _rotated = registration.retired_binding;
                self.set_terminal_live_binding(Some(registration.record.binding_id));
                Ok(())
            }
            Err(error) => {
                let _retired = error.retired_binding;
                self.set_terminal_live_binding(None);
                Err(format!("{error:#}"))
            }
        }
    }

    pub(super) fn drain_live_bridge(&mut self, context: &egui::Context) {
        let interaction = self.live_interaction_state();
        let Some(registry) = &mut self.live_bridge else {
            return;
        };
        let (report, retired) =
            registry.drain(CATALOG_BINDING_KEY, &mut self.workspace, interaction);
        let pending = registry.has_pending();
        let _ = registry;
        if retired.is_some() {
            self.set_terminal_live_binding(None);
        }

        if let Some(refresh) = live_refresh_kind(report) {
            self.draft_id = None;
            self.sync_draft();
            self.invalidate_selected();
            self.history_selected = None;
            match refresh {
                LiveRefreshKind::Applied => {
                    self.status = "Applied live agent changes".into();
                    self.error = false;
                }
                LiveRefreshKind::OutcomeUnknown => {
                    self.status =
                        "Live agent outcome is unknown; inspect current history before retrying"
                            .into();
                    self.error = true;
                }
                LiveRefreshKind::ReopenRequired => {
                    self.status =
                        "Live bridge was poisoned after an uncertain outcome; reopen the catalog"
                            .into();
                    self.error = true;
                }
            }
            context.request_repaint();
        }
        if pending {
            context.request_repaint();
        }
    }

    pub(super) fn observe_live_bridge(&mut self) {
        let interaction = self.live_interaction_state();
        let Some(registry) = &mut self.live_bridge else {
            return;
        };
        if let Err(error) = registry.observe(CATALOG_BINDING_KEY, &self.workspace, interaction) {
            self.status = format!("Could not publish Lumen live state: {error:#}");
            self.error = true;
        }
    }

    fn live_interaction_state(&self) -> LumenLiveInteractionState {
        let active = self.adjustment_interacting
            || self.crop_drag.is_some()
            || self.spot_stroke_start.is_some()
            || self.export_open
            || self.reset_confirmation
            || self.remove_confirmation
            || self.rename_batch.is_some()
            || self.pending_catalog_switch.is_some();
        if active {
            LumenLiveInteractionState::Active
        } else {
            LumenLiveInteractionState::Idle
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uncertain_changed_outcome_uses_the_error_refresh_path() {
        assert_eq!(
            live_refresh_kind(LumenLiveDrainReport {
                workspace_changed: true,
                outcome_unknown: true,
                ..LumenLiveDrainReport::default()
            }),
            Some(LiveRefreshKind::OutcomeUnknown)
        );
        assert_eq!(
            live_refresh_kind(LumenLiveDrainReport {
                reopen_required: true,
                ..LumenLiveDrainReport::default()
            }),
            Some(LiveRefreshKind::ReopenRequired)
        );
    }
}
