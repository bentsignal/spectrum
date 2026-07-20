use std::{
    sync::mpsc::{self, Receiver, Sender},
    time::{Duration, Instant},
};

use prism_core::ProjectHistory;
use spectrum_history_ui::{
    HistoryGraph, HistoryTheme, history_header, history_tree, revision_details,
};
use spectrum_revisions::RevisionId;

use super::*;

const REFRESH_INTERVAL: Duration = Duration::from_millis(500);

fn prism_history_theme() -> HistoryTheme {
    HistoryTheme {
        ink: INK,
        panel: PANEL,
        surface: SURFACE,
        hover_surface: HOVER_SURFACE,
        active_surface: ACTIVE_SURFACE,
        focus_surface: SELECTED_SURFACE,
        border: BORDER,
        text: TEXT,
        muted: MUTED,
        accent: ACCENT,
        human: Color32::from_rgb(126, 151, 186),
        agent: ACCENT,
        system: ACCENT_WARM,
    }
}

pub(super) struct HistoryViewState {
    pub(super) visible: bool,
    history: Option<ProjectHistory>,
    selected: Option<RevisionId>,
    preview_revision: Option<RevisionId>,
    preview: Option<TextureHandle>,
    preview_error: Option<String>,
    preview_sender: Sender<HistoryPreviewRequest>,
    preview_receiver: Receiver<HistoryPreviewResult>,
    preview_pending: Option<RevisionId>,
    refresh_at: Instant,
    scroll_to_current: bool,
}

impl HistoryViewState {
    pub(super) fn new(repaint: egui::Context) -> Self {
        let (preview_sender, request_receiver) = mpsc::channel();
        let (result_sender, preview_receiver) = mpsc::channel();
        spawn_history_preview_worker(request_receiver, result_sender, repaint);
        Self {
            visible: false,
            history: None,
            selected: None,
            preview_revision: None,
            preview: None,
            preview_error: None,
            preview_sender,
            preview_receiver,
            preview_pending: None,
            refresh_at: Instant::now(),
            scroll_to_current: false,
        }
    }

    pub(super) fn mark_stale(&mut self) {
        self.refresh_at = Instant::now();
    }

    pub(super) fn workspace_changed(&mut self) {
        self.history = None;
        self.selected = None;
        self.preview_revision = None;
        self.preview = None;
        self.preview_pending = None;
        self.scroll_to_current = true;
        self.mark_stale();
    }
}

impl PrismApp {
    pub(super) fn toggle_history(&mut self) {
        if self.history.visible {
            self.history.visible = false;
            self.reset_canvas_cache();
            self.status = "Returned to the canvas".into();
            self.status_error = false;
            return;
        }
        self.finish_interaction();
        self.history.visible = true;
        self.history.selected = None;
        self.history.scroll_to_current = true;
        self.refresh_history(true);
        if self.history.history.is_some() {
            self.status = "Opened revision history".into();
            self.status_error = false;
        } else {
            self.history.visible = false;
        }
    }

    pub(super) fn history_view(&mut self, root: &mut egui::Ui) {
        let theme = prism_history_theme();
        let context = root.ctx().clone();
        self.receive_history_preview(&context);
        self.refresh_history(false);
        self.history_details_panel(root);
        let mut clicked = None;
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(INK).inner_margin(12))
            .show(root, |ui| {
                if history_header(ui, "Every completed action remains reachable", theme) {
                    self.history.mark_stale();
                }
                ui.add_space(8.0);
                ui.separator();
                ui.add_space(6.0);
                if let Some(history) = self.history.history.as_ref() {
                    let graph = HistoryGraph {
                        root: history.root,
                        current: history.current,
                        revisions: &history.revisions,
                        sessions: &history.sessions,
                    };
                    clicked = history_tree(
                        ui,
                        graph,
                        self.history.selected,
                        &mut self.history.scroll_to_current,
                        "prism-revision-tree-scroll",
                        theme,
                    );
                } else {
                    ui.centered_and_justified(|ui| {
                        ui.label(RichText::new("History is unavailable").color(MUTED));
                    });
                }
            });
        if let Some(target) = clicked {
            self.navigate_history(target);
        }
    }

    fn history_details_panel(&mut self, root: &mut egui::Ui) {
        let mut close = false;
        egui::Panel::right("prism-history-details")
            .default_size(328.0)
            .min_size(328.0)
            .max_size(328.0)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(PANEL)
                    .inner_margin(12)
                    .stroke(Stroke::new(1.0, BORDER)),
            )
            .show(root, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("HISTORY").size(11.0).strong().color(TEXT));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if quiet_button(ui, "Canvas  ⌘H").clicked() {
                            close = true;
                        }
                    });
                });
                ui.add_space(10.0);
                self.history_preview(ui);
                ui.add_space(12.0);
                self.history_revision_details(ui);
            });
        if close {
            self.toggle_history();
        }
    }

    fn history_preview(&self, ui: &mut egui::Ui) {
        let width = ui.available_width();
        let frame = egui::Frame::new()
            .fill(INK)
            .corner_radius(RADIUS)
            .stroke(Stroke::new(1.0, BORDER))
            .inner_margin(6);
        frame.show(ui, |ui| {
            ui.allocate_ui_with_layout(
                Vec2::new(width - 18.0, 210.0),
                egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
                |ui| {
                    if let Some(texture) = &self.history.preview {
                        let source = texture.size_vec2();
                        let available = Vec2::new(ui.available_width(), 194.0);
                        let scale = (available.x / source.x)
                            .min(available.y / source.y)
                            .min(1.0);
                        let size = source * scale;
                        ui.add(egui::Image::new((texture.id(), size)));
                    } else {
                        ui.label(
                            RichText::new(
                                self.history
                                    .preview_error
                                    .as_deref()
                                    .unwrap_or("Rendering revision preview…"),
                            )
                            .size(11.0)
                            .color(MUTED),
                        );
                    }
                },
            );
        });
    }

    fn history_revision_details(&self, ui: &mut egui::Ui) {
        let Some(history) = &self.history.history else {
            return;
        };
        revision_details(
            ui,
            HistoryGraph {
                root: history.root,
                current: history.current,
                revisions: &history.revisions,
                sessions: &history.sessions,
            },
            self.history.selected,
            prism_history_theme(),
        );
    }

    fn navigate_history(&mut self, target: RevisionId) {
        match self.workspace.move_to_revision(target) {
            Ok(changed) => {
                self.history.selected = None;
                self.history.mark_stale();
                self.refresh_history(true);
                if changed {
                    self.reset_canvas_cache();
                    self.status = "Moved to an existing revision".into();
                } else {
                    self.status = "Already at this revision".into();
                }
                self.status_error = false;
            }
            Err(error) => {
                self.status = format!("Could not navigate history: {error:#}");
                self.status_error = true;
            }
        }
    }

    fn refresh_history(&mut self, force: bool) {
        let now = Instant::now();
        if !force && now < self.history.refresh_at {
            return;
        }
        self.history.refresh_at = now + REFRESH_INTERVAL;
        match self.workspace.history() {
            Ok(Some(history)) => {
                let selected = self
                    .history
                    .selected
                    .filter(|selected| history.revisions.iter().any(|item| item.id == *selected))
                    .unwrap_or(history.current);
                let preview_changed = preview_request_needed(
                    self.history.preview_revision,
                    self.history.preview_pending,
                    selected,
                );
                self.history.selected = Some(selected);
                self.history.history = Some(history);
                if preview_changed {
                    self.refresh_history_preview(selected);
                }
            }
            Ok(None) => {
                self.history.history = None;
                self.status = "Convert this legacy document before viewing revision history".into();
                self.status_error = true;
            }
            Err(error) => {
                self.history.history = None;
                self.status = format!("Could not load revision history: {error:#}");
                self.status_error = true;
            }
        }
    }

    fn refresh_history_preview(&mut self, revision: RevisionId) {
        self.history.preview_error = None;
        self.history.preview_pending = Some(revision);
        if self
            .history
            .preview_sender
            .send(HistoryPreviewRequest {
                revision,
                document: self.workspace.document.clone(),
            })
            .is_err()
        {
            self.history.preview_error = Some("History preview worker stopped".into());
            self.history.preview_pending = None;
        }
    }

    fn receive_history_preview(&mut self, context: &egui::Context) {
        while let Ok(result) = self.history.preview_receiver.try_recv() {
            if self.history.preview_pending != Some(result.revision) {
                continue;
            }
            self.history.preview_pending = None;
            match result.result {
                Ok(image) => {
                    let rgba = image.to_rgba8();
                    let size = [rgba.width() as usize, rgba.height() as usize];
                    let pixels = egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
                    self.history.preview = Some(context.load_texture(
                        format!("prism-history-preview-{}", result.revision),
                        pixels,
                        TextureOptions::LINEAR,
                    ));
                    self.history.preview_revision = Some(result.revision);
                }
                Err(error) => {
                    self.history.preview_error = Some(error);
                    self.history.preview_revision = None;
                }
            }
        }
    }
}

struct HistoryPreviewRequest {
    revision: RevisionId,
    document: Document,
}

struct HistoryPreviewResult {
    revision: RevisionId,
    result: Result<image::DynamicImage, String>,
}

fn spawn_history_preview_worker(
    receiver: Receiver<HistoryPreviewRequest>,
    sender: Sender<HistoryPreviewResult>,
    repaint: egui::Context,
) {
    std::thread::spawn(move || {
        while let Ok(request) = receiver.recv() {
            let result = prism_core::render_document_thumbnail(&request.document, 640)
                .map_err(|error| format!("{error:#}"));
            let _ = sender.send(HistoryPreviewResult {
                revision: request.revision,
                result,
            });
            repaint.request_repaint();
        }
    });
}

fn preview_request_needed(
    displayed: Option<RevisionId>,
    pending: Option<RevisionId>,
    selected: RevisionId,
) -> bool {
    displayed != Some(selected) && pending != Some(selected)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_preview_is_kept_until_its_replacement_is_ready() {
        let old = RevisionId::from_bytes([1; 16]);
        let next = RevisionId::from_bytes([2; 16]);
        assert!(preview_request_needed(Some(old), None, next));
        assert!(!preview_request_needed(Some(old), Some(next), next));
        assert!(!preview_request_needed(Some(next), None, next));
    }
}
