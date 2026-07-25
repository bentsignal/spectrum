use prism_core::PrismLiveInteractionState;

use super::*;

impl PrismApp {
    pub(super) fn register_live_tab(&mut self, tab_id: u64) {
        let workspace = if tab_id == self.active_tab_id {
            &self.workspace
        } else {
            let Some(workspace) = self.inactive_workspaces.get(&tab_id) else {
                return;
            };
            workspace
        };
        let Some(registry) = &mut self.live_bridge else {
            return;
        };
        match registry.register(tab_id, workspace) {
            Ok(registration) => {
                if let Some(retired) = registration.retired_binding {
                    let affected = self.terminal.live_binding_rotated(retired);
                    if affected > 0 {
                        self.status = format!(
                            "Project binding changed; open a new terminal session ({affected} stale)"
                        );
                        self.status_error = false;
                    }
                }
                debug_assert_eq!(
                    registry.record(tab_id).map(|record| record.binding_id),
                    Some(registration.record.binding_id)
                );
            }
            Err(error) => {
                self.status = format!("Could not publish live project: {error:#}");
                self.status_error = true;
            }
        }
    }

    pub(super) fn remove_live_tab(&mut self, tab_id: u64) {
        if let Some(registry) = &mut self.live_bridge
            && let Some(retired) = registry.remove(tab_id)
        {
            self.terminal.live_project_closed(retired);
        }
    }

    pub(super) fn rotate_active_live_binding(&mut self) {
        let tab_id = self.active_tab_id;
        self.register_live_tab(tab_id);
    }

    pub(super) fn drain_live_bridges(&mut self, context: &egui::Context) {
        let interaction = self.live_interaction_state();
        let Some(registry) = &mut self.live_bridge else {
            return;
        };
        let order = registry.ordered_tabs(&self.tab_ids);
        let mut active_applied = false;
        let mut inactive_applied = Vec::new();
        for tab_id in order {
            let report = if tab_id == self.active_tab_id {
                registry.drain(tab_id, &mut self.workspace, interaction)
            } else {
                let Some(workspace) = self.inactive_workspaces.get_mut(&tab_id) else {
                    continue;
                };
                registry.drain(tab_id, workspace, PrismLiveInteractionState::Idle)
            };
            if report.applied > 0 {
                if tab_id == self.active_tab_id {
                    active_applied = true;
                } else {
                    inactive_applied.push(tab_id);
                }
                break;
            }
        }
        let pending = registry.has_pending();
        let _ = registry;

        if active_applied {
            self.finish_durable_revision_advance();
            self.apply_canvas_invalidation(CanvasInvalidation::All);
            self.sync_active_raster_sources();
            self.history.mark_stale();
            self.status = "Applied live agent changes".into();
            self.status_error = false;
            context.request_repaint();
        }
        for tab_id in inactive_applied {
            if let Some(workspace) = self.inactive_workspaces.get(&tab_id) {
                self.raster_sources
                    .set_tab_document(tab_id, &workspace.document);
            }
        }
        if pending {
            context.request_repaint();
        }
    }

    pub(super) fn observe_live_bridges(&mut self) {
        let active_interaction = self.live_interaction_state();
        let Some(registry) = &mut self.live_bridge else {
            return;
        };
        let mut error = None;
        if let Err(observe_error) =
            registry.observe(self.active_tab_id, &self.workspace, active_interaction)
        {
            error = Some(observe_error);
        }
        for (tab_id, workspace) in &self.inactive_workspaces {
            if let Err(observe_error) =
                registry.observe(*tab_id, workspace, PrismLiveInteractionState::Idle)
            {
                error.get_or_insert(observe_error);
            }
        }
        if let Some(error) = error {
            self.status = format!("Could not publish Prism live state: {error:#}");
            self.status_error = true;
        }
    }

    pub(super) fn live_binding_record(
        &self,
        tab_id: u64,
    ) -> Option<spectrum_live_bridge::DiscoveryRecord> {
        self.live_bridge.as_ref()?.record(tab_id)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn default_drain_report_has_no_mutation() {
        assert_eq!(prism_core::PrismLiveDrainReport::default().applied, 0);
    }
}
