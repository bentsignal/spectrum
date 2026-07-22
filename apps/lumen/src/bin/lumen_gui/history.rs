use spectrum_history_ui::{
    HistoryGraph, HistoryTheme, history_header, history_tree, revision_details,
};
use spectrum_revisions::RevisionId;

use super::*;

impl LumenApp {
    pub(super) fn toggle_history(&mut self) {
        self.history_open = !self.history_open;
        if self.history_open {
            self.history_selected = None;
            self.history_scroll_to_current = true;
            self.status = "Opened photo revision history".into();
        } else {
            self.status = "Returned to the photo".into();
        }
        self.error = false;
    }

    pub(super) fn history_view(&mut self, root: &mut egui::Ui) {
        let context = root.ctx().clone();
        self.ensure_preview(&context);
        let history = match self.workspace.history() {
            Ok(Some(history)) => history,
            Ok(None) => {
                self.history_open = false;
                self.status = "This legacy catalog does not have durable photo history".into();
                self.error = true;
                return;
            }
            Err(error) => {
                self.history_open = false;
                self.status = format!("Could not load history: {error:#}");
                self.error = true;
                return;
            }
        };
        let theme = HistoryTheme::default();
        self.history_selected = self
            .history_selected
            .filter(|selected| history.revisions.iter().any(|item| item.id == *selected))
            .or(Some(history.current));
        let graph = HistoryGraph {
            root: history.root,
            current: history.current,
            revisions: &history.revisions,
            sessions: &history.sessions,
        };

        let mut close = false;
        egui::Panel::right("lumen-history-details")
            .default_size(360.0)
            .min_size(360.0)
            .max_size(360.0)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(theme.panel)
                    .inner_margin(18)
                    .stroke(Stroke::new(1.0, theme.border)),
            )
            .show(root, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("HISTORY")
                            .size(14.0)
                            .strong()
                            .color(theme.text),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .button(format!("Photo  {}", history_shortcut_label()))
                            .clicked()
                        {
                            close = true;
                        }
                    });
                });
                ui.add_space(14.0);
                self.history_preview(ui, theme);
                ui.add_space(16.0);
                revision_details(ui, graph, self.history_selected, theme);
            });

        let mut clicked = None;
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(theme.ink).inner_margin(18))
            .show(root, |ui| {
                if history_header(
                    ui,
                    &format!(
                        "Photo {} · every completed edit remains reachable",
                        history.photo_id
                    ),
                    theme,
                ) {
                    self.status = "Refreshed photo revision history".into();
                    self.error = false;
                }
                ui.add_space(12.0);
                ui.separator();
                ui.add_space(8.0);
                clicked = history_tree(
                    ui,
                    graph,
                    self.history_selected,
                    &mut self.history_scroll_to_current,
                    ("lumen-revision-tree-scroll", history.track_id),
                    theme,
                );
            });
        if close {
            self.toggle_history();
        } else if let Some(target) = clicked {
            self.navigate_history(target);
        }
    }

    fn history_preview(&self, ui: &mut egui::Ui, theme: HistoryTheme) {
        let width = ui.available_width();
        egui::Frame::new()
            .fill(theme.ink)
            .corner_radius(8)
            .stroke(Stroke::new(1.0, theme.border))
            .inner_margin(8)
            .show(ui, |ui| {
                ui.allocate_ui_with_layout(
                    Vec2::new(width - 18.0, 210.0),
                    egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
                    |ui| {
                        if let Some(texture) = &self.preview {
                            let source = texture.size_vec2();
                            let available = Vec2::new(ui.available_width(), 194.0);
                            let scale = (available.x / source.x)
                                .min(available.y / source.y)
                                .min(1.0);
                            ui.add(egui::Image::new((texture.id(), source * scale)));
                        } else {
                            ui.label(
                                RichText::new("Rendering photo preview…")
                                    .size(11.0)
                                    .color(theme.muted),
                            );
                        }
                    },
                );
            });
    }

    fn navigate_history(&mut self, target: RevisionId) {
        match self.workspace.move_to_revision(target) {
            Ok(true) => {
                self.history_selected = Some(target);
                self.reset_catalog_view(self.library_mode);
                self.status = "Moved this photo to the selected revision".into();
                self.error = false;
            }
            Ok(false) => {
                self.history_selected = Some(target);
                self.status = "Already at this revision".into();
                self.error = false;
            }
            Err(error) => {
                self.status = format!("Could not navigate history: {error:#}");
                self.error = true;
            }
        }
    }
}
