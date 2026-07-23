use super::*;

impl LumenApp {
    pub(super) fn canvas(&mut self, root: &mut egui::Ui) {
        let context = root.ctx().clone();
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(CANVAS).inner_margin(16))
            .show(root, |ui| {
                if self.preview.is_some() {
                    let available = ui.available_size();
                    let (rect, response) =
                        ui.allocate_exact_size(available, Sense::click_and_drag());
                    if response.dragged() && !self.crop_mode && !self.spot_mode {
                        self.pan += response.drag_delta();
                    }
                    if response.hovered() && !self.crop_mode && !self.spot_mode {
                        let scroll = ui.input(|input| input.smooth_scroll_delta.y);
                        if scroll.abs() > 0.1 {
                            self.zoom = (self.zoom * (scroll * 0.0018).exp()).clamp(0.25, 8.0);
                        }
                    }
                    self.update_preview_resolution(available, context.pixels_per_point());
                    self.ensure_preview(&context);
                    let Some(texture) = self.preview.clone() else {
                        return;
                    };
                    if self.compare_mode == CompareMode::SideBySide
                        && let Some(original) = self.original_preview.clone()
                    {
                        let gap = 10.0;
                        let half = (rect.width() - gap) * 0.5;
                        let before_rect =
                            Rect::from_min_size(rect.min, Vec2::new(half, rect.height()));
                        let after_rect = Rect::from_min_size(
                            Pos2::new(before_rect.right() + gap, rect.top()),
                            Vec2::new(half, rect.height()),
                        );
                        ui.painter()
                            .rect_filled(before_rect, 6.0, Color32::from_rgb(18, 20, 23));
                        ui.painter()
                            .rect_filled(after_rect, 6.0, Color32::from_rgb(18, 20, 23));
                        let before_size =
                            fit_size(original.size_vec2(), before_rect.size()) * self.zoom;
                        let after_size = preview_fit_size(
                            self.preview_layout_size,
                            texture.size_vec2(),
                            after_rect.size(),
                        ) * self.zoom;
                        let before_image =
                            Rect::from_center_size(before_rect.center() + self.pan, before_size);
                        let after_image =
                            Rect::from_center_size(after_rect.center() + self.pan, after_size);
                        paint_texture(ui, before_rect, &original, before_image);
                        paint_texture(ui, after_rect, &texture, after_image);
                        compare_label(ui, before_rect, "ORIGINAL");
                        compare_label(ui, after_rect, "EDITED");
                        paint_preview_quality(
                            ui,
                            rect,
                            self.preview_fast,
                            self.adjustment_interacting,
                        );
                        return;
                    }
                    let base = preview_fit_size(
                        self.preview_layout_size,
                        texture.size_vec2(),
                        rect.size(),
                    );
                    let size = base * self.zoom;
                    let image_rect = Rect::from_center_size(rect.center() + self.pan, size);
                    ui.painter().with_clip_rect(rect).image(
                        texture.id(),
                        image_rect,
                        Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                        Color32::WHITE,
                    );
                    if self.crop_mode {
                        crop_interaction(
                            ui,
                            &response,
                            image_rect,
                            &mut self.crop_draft,
                            &mut self.crop_drag,
                        );
                        paint_crop_overlay(ui, rect, image_rect, self.crop_draft);
                    } else if self.spot_mode {
                        if let Some(id) = self.workspace.project.selected {
                            let (changed, commit) = spot_interaction(
                                &response,
                                image_rect,
                                &mut self.draft.spots,
                                self.spot_radius,
                                &mut self.spot_stroke_start,
                            );
                            if changed {
                                self.preview = None;
                                ui.ctx().request_repaint();
                            }
                            if commit {
                                self.draft = self.draft.clone().sanitized();
                                self.finish_edit(id);
                            }
                        }
                        paint_spot_overlay(
                            ui,
                            rect,
                            image_rect,
                            &self.draft.spots,
                            self.spot_radius,
                        );
                    } else if self.zoom > 1.01 {
                        ui.painter().text(
                            rect.left_bottom() + Vec2::new(8.0, -8.0),
                            egui::Align2::LEFT_BOTTOM,
                            "Drag to pan | Scroll to zoom",
                            egui::FontId::proportional(11.0),
                            Color32::from_gray(150),
                        );
                    }
                    paint_preview_quality(ui, rect, self.preview_fast, self.adjustment_interacting);
                } else {
                    let available = ui.available_rect_before_wrap();
                    ui.allocate_rect(available, Sense::hover());
                    let content = Rect::from_center_size(
                        available.center(),
                        Vec2::new(available.width().min(560.0), 190.0),
                    );
                    ui.scope_builder(
                        egui::UiBuilder::new()
                            .max_rect(content)
                            .layout(egui::Layout::top_down(egui::Align::Center)),
                        |ui| {
                            ui.add_space(10.0);
                            ui.label(RichText::new("L U M E N").size(12.0).strong().color(ACCENT));
                            ui.add_space(14.0);
                            ui.label(
                                RichText::new("Bring a shoot into focus")
                                    .size(26.0)
                                    .color(Color32::from_gray(226)),
                            );
                            ui.add_space(8.0);
                            ui.label(
                                RichText::new("Drop photos here, or start by choosing a shoot.")
                                    .size(13.0)
                                    .color(Color32::from_gray(145)),
                            );
                            ui.add_space(16.0);
                            ui.horizontal(|ui| {
                                if ui.button("Choose Photos").clicked() {
                                    self.import_dialog();
                                }
                                if ui.button("Open Catalog").clicked() {
                                    self.open_catalog();
                                }
                            });
                        },
                    );
                }
            });
    }
}

fn paint_preview_quality(ui: &egui::Ui, canvas: Rect, degraded: bool, interacting: bool) {
    if !degraded {
        return;
    }
    let label = if interacting {
        "Interactive preview · Full quality on release"
    } else {
        "Refining full quality…"
    };
    let badge = Rect::from_min_size(
        canvas.right_top() + Vec2::new(-250.0, 10.0),
        Vec2::new(240.0, 26.0),
    );
    ui.painter()
        .rect_filled(badge, 6.0, Color32::from_black_alpha(205));
    ui.painter().text(
        badge.center(),
        egui::Align2::CENTER_CENTER,
        label,
        egui::FontId::proportional(10.5),
        Color32::from_gray(205),
    );
}
