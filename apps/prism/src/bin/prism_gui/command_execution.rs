use prism_core::Command;

use super::{PrismApp, canvas_invalidation, text_geometry_invalidation};

impl PrismApp {
    pub(super) fn execute(&mut self, command: Command) -> bool {
        self.clear_brush_for_document_mutation();
        self.clear_font_hover_preview();
        if self.inline_text_editor.is_some() && !matches!(command, Command::SelectLayer { .. }) {
            self.settle_inline_text_editor();
        }
        let invalidation = canvas_invalidation(&command);
        let text_geometry_id = text_geometry_invalidation(&command);
        match self.workspace.execute(command) {
            Ok(output) => {
                self.apply_canvas_invalidation(invalidation);
                if let Some(id) = text_geometry_id {
                    self.text_source_geometries.remove(id);
                }
                self.sync_active_raster_sources();
                if let Some(error) = self.workspace.pending_publish_error() {
                    self.status = format!(
                        "Edit is safe in Prism recovery storage, but the project file could not update: {error}"
                    );
                    self.status_error = true;
                } else {
                    self.status = if output.warnings.is_empty() {
                        output.message
                    } else {
                        format!(
                            "{} — Warning: {}",
                            output.message,
                            output.warnings.join(" ")
                        )
                    };
                    self.status_error = false;
                }
                self.history.mark_stale();
                true
            }
            Err(error) => {
                self.status = format!("{error:#}");
                self.status_error = true;
                false
            }
        }
    }

    pub(super) fn execute_batch(&mut self, commands: Vec<Command>) -> bool {
        self.clear_brush_for_document_mutation();
        self.clear_font_hover_preview();
        self.settle_inline_text_editor();
        let invalidations = commands.iter().map(canvas_invalidation).collect::<Vec<_>>();
        let text_geometry_ids = commands
            .iter()
            .filter_map(text_geometry_invalidation)
            .collect::<Vec<_>>();
        match self.workspace.execute_batch(commands) {
            Ok(outputs) => {
                for invalidation in invalidations {
                    self.apply_canvas_invalidation(invalidation);
                }
                for id in text_geometry_ids {
                    self.text_source_geometries.remove(id);
                }
                self.sync_active_raster_sources();
                if let Some(error) = self.workspace.pending_publish_error() {
                    self.status = format!(
                        "Edit is safe in Prism recovery storage, but the project file could not update: {error}"
                    );
                    self.status_error = true;
                } else {
                    self.status = outputs
                        .last()
                        .map(|output| output.message.clone())
                        .unwrap_or_else(|| "Completed edit batch".into());
                    self.status_error = false;
                }
                self.history.mark_stale();
                true
            }
            Err(error) => {
                self.status = format!("{error:#}");
                self.status_error = true;
                false
            }
        }
    }

    pub(super) fn preview_command(&mut self, command: Command) -> bool {
        self.clear_brush_for_document_mutation();
        let invalidation = canvas_invalidation(&command);
        let text_geometry_id = text_geometry_invalidation(&command);
        match self.workspace.preview(command) {
            Ok(_) => {
                self.apply_canvas_invalidation(invalidation);
                if let Some(id) = text_geometry_id {
                    self.text_source_geometries.remove(id);
                }
                true
            }
            Err(error) => {
                self.status = format!("{error:#}");
                self.status_error = true;
                false
            }
        }
    }

    pub(super) fn preview_commands(&mut self, commands: Vec<Command>) -> bool {
        self.clear_brush_for_document_mutation();
        let invalidations = commands.iter().map(canvas_invalidation).collect::<Vec<_>>();
        let text_geometry_ids = commands
            .iter()
            .filter_map(text_geometry_invalidation)
            .collect::<Vec<_>>();
        match self.workspace.preview_batch(commands) {
            Ok(_) => {
                for invalidation in invalidations {
                    self.apply_canvas_invalidation(invalidation);
                }
                for id in text_geometry_ids {
                    self.text_source_geometries.remove(id);
                }
                true
            }
            Err(error) => {
                self.status = format!("{error:#}");
                self.status_error = true;
                false
            }
        }
    }
}
