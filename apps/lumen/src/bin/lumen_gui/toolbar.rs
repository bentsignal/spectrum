use super::*;
use crate::terminal::terminal_shortcut_label;

impl LumenApp {
    pub(super) fn toolbar(&mut self, root: &mut egui::Ui) {
        #[cfg(not(target_os = "macos"))]
        let recent_catalogs = self.recent_catalogs.clone();
        egui::Panel::top("toolbar")
            .frame(
                egui::Frame::new()
                    .fill(PANEL)
                    .stroke(Stroke::new(1.0, Color32::from_gray(49)))
                    .inner_margin(egui::Margin::symmetric(14, 9)),
            )
            .show(root, |ui| {
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    let (brand_rect, _) =
                        ui.allocate_exact_size(Vec2::new(54.0, 28.0), Sense::hover());
                    ui.painter().text(
                        brand_rect.center(),
                        egui::Align2::CENTER_CENTER,
                        "LUMEN",
                        egui::FontId::proportional(15.0),
                        ACCENT,
                    );
                    let (divider_rect, _) =
                        ui.allocate_exact_size(Vec2::new(1.0, 28.0), Sense::hover());
                    ui.painter().line_segment(
                        [
                            divider_rect.center() - Vec2::new(0.0, 9.0),
                            divider_rect.center() + Vec2::new(0.0, 9.0),
                        ],
                        Stroke::new(1.0, Color32::from_gray(58)),
                    );
                    ui.add_space(2.0);
                    if self.terminal.visible() {
                        ui.label(RichText::new("TERMINAL").strong().color(ACCENT));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui
                                .button(format!("Back to Photos  {}", terminal_shortcut_label()))
                                .clicked()
                            {
                                self.toggle_terminal();
                            }
                            ui.label(
                                RichText::new(current_catalog_name(&self.workspace))
                                    .color(Color32::GRAY),
                            );
                        });
                        return;
                    }
                    #[cfg(not(target_os = "macos"))]
                    {
                        if ui
                            .add_enabled(
                                !self.history_open,
                                egui::Button::new(format!(
                                    "Terminal  {}",
                                    terminal_shortcut_label()
                                )),
                            )
                            .clicked()
                        {
                            self.toggle_terminal();
                        }
                        if ui.button("Import Photos").clicked() {
                            self.import_dialog();
                        }
                        ui.menu_button("Catalog", |ui| {
                            if ui.button("New Catalog").clicked() {
                                ui.close();
                                self.new_catalog();
                            }
                            if ui.button("Open Catalog...").clicked() {
                                ui.close();
                                self.open_catalog();
                            }
                            if ui.button("Move Catalog...").clicked() {
                                ui.close();
                                self.move_project();
                            }
                            if ui.button("History  Ctrl+H").clicked() {
                                ui.close();
                                self.history_open = true;
                            }
                            ui.separator();
                            ui.label(
                                RichText::new("RECENT CATALOGS")
                                    .size(10.0)
                                    .color(Color32::GRAY),
                            );
                            if recent_catalogs.is_empty() {
                                ui.label(RichText::new("None yet").color(Color32::GRAY));
                            } else {
                                for path in &recent_catalogs {
                                    if ui
                                        .add_enabled(
                                            path.exists(),
                                            egui::Button::new(catalog_label(path)),
                                        )
                                        .on_hover_text(path.display().to_string())
                                        .clicked()
                                    {
                                        self.request_catalog_switch(CatalogSwitch::Open(
                                            path.clone(),
                                        ));
                                        ui.close();
                                    }
                                }
                            }
                        });
                    }
                    if self.library_mode {
                        ui.separator();
                        ui.label(RichText::new("ALL SHOOTS").strong().color(ACCENT));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(current_catalog_name(&self.workspace))
                                    .color(Color32::GRAY),
                            );
                        });
                        return;
                    }
                    #[cfg(not(target_os = "macos"))]
                    {
                        if ui.button("All Shoots").clicked() {
                            self.library_mode = true;
                        }
                        ui.separator();
                        let (can_back, can_forward) =
                            (self.workspace.can_undo(), self.workspace.can_redo());
                        if ui
                            .add_enabled(can_back, egui::Button::new("Undo Edit"))
                            .on_hover_text("Undo the last photo edit (Cmd/Ctrl+Z)")
                            .clicked()
                            && self.execute(Command::Undo)
                        {
                            self.draft_id = None;
                            self.sync_draft();
                        }
                        if ui
                            .add_enabled(can_forward, egui::Button::new("Redo Edit"))
                            .on_hover_text("Redo the last photo edit (Cmd/Ctrl+Shift+Z)")
                            .clicked()
                            && self.execute(Command::Redo)
                        {
                            self.draft_id = None;
                            self.sync_draft();
                        }
                    }
                    ui.separator();
                    if ui.button("Fit").clicked() {
                        self.zoom = 1.0;
                        self.pan = Vec2::ZERO;
                    }
                    if ui.button("-").clicked() {
                        self.zoom = (self.zoom / 1.25).max(0.25);
                    }
                    ui.label(format!("{:.0}%", self.zoom * 100.0));
                    if ui.button("+").clicked() {
                        self.zoom = (self.zoom * 1.25).min(8.0);
                    }
                    ui.separator();
                    ui.selectable_value(&mut self.compare_mode, CompareMode::Edited, "Edited");
                    ui.selectable_value(
                        &mut self.compare_mode,
                        CompareMode::SideBySide,
                        "Before | After",
                    )
                    .on_hover_text("Compare original and edited photo (C)");
                    ui.separator();
                    let pick = self
                        .workspace
                        .project
                        .selected_photo()
                        .map(|photo| photo.pick)
                        .unwrap_or_default();
                    if ui
                        .selectable_label(pick == PickState::Keep, "Keep")
                        .on_hover_text("Mark selected photos as keeps (P)")
                        .clicked()
                    {
                        self.set_pick(
                            self.selected_photo_ids(),
                            toggled_pick(pick, PickState::Keep),
                        );
                    }
                    if ui
                        .selectable_label(pick == PickState::Reject, "Reject")
                        .on_hover_text("Mark selected photos as rejects (X)")
                        .clicked()
                    {
                        self.set_pick(
                            self.selected_photo_ids(),
                            toggled_pick(pick, PickState::Reject),
                        );
                    }
                    if ui
                        .add_enabled(
                            self.workspace.project.selected.is_some(),
                            egui::Button::new("Remove"),
                        )
                        .on_hover_text("Remove selected photos from this catalog")
                        .clicked()
                    {
                        self.remove_confirmation = true;
                    }
                    ui.separator();
                    if self.crop_mode {
                        if ui
                            .button(RichText::new("Apply Crop").color(ACCENT))
                            .clicked()
                        {
                            self.apply_crop();
                        }
                        if ui.button("Cancel").clicked() {
                            self.cancel_crop();
                        }
                    } else if ui
                        .add_enabled(
                            self.workspace.project.selected.is_some(),
                            egui::Button::new("Crop"),
                        )
                        .clicked()
                    {
                        self.begin_crop();
                    }
                    #[cfg(not(target_os = "macos"))]
                    {
                        if ui
                            .add_enabled(
                                self.workspace.project.selected.is_some(),
                                egui::Button::new(format!(
                                    "Export{}",
                                    match self.selected_photo_ids().len() {
                                        0 | 1 => String::new(),
                                        count => format!(" {count}"),
                                    }
                                )),
                            )
                            .clicked()
                        {
                            self.open_export();
                        }
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(current_catalog_name(&self.workspace))
                                .color(Color32::GRAY),
                        );
                    });
                });
            });
    }

    pub(super) fn filmstrip(&mut self, root: &mut egui::Ui) {
        let context = root.ctx().clone();
        egui::Panel::left("filmstrip")
            .resizable(true)
            .default_size(172.0)
            .size_range(138.0..=240.0)
            .frame(
                egui::Frame::new()
                    .fill(PANEL)
                    .inner_margin(egui::Margin::symmetric(10, 10)),
            )
            .show(root, |ui| {
                let batch_photo_count = self
                    .workspace
                    .project
                    .photos
                    .iter()
                    .filter(|photo| {
                        self.active_batch
                            .is_none_or(|batch_id| photo.batch_id == Some(batch_id))
                    })
                    .count();
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("PHOTOS")
                            .strong()
                            .size(12.0)
                            .color(Color32::GRAY),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let selected = self.selected_photo_ids().len();
                        ui.label(if selected > 1 {
                            format!("{selected} selected")
                        } else {
                            batch_photo_count.to_string()
                        });
                    });
                });
                ui.label(
                    RichText::new("Shift for range | Cmd/Ctrl to toggle")
                        .size(9.0)
                        .color(Color32::from_gray(115)),
                );
                if let Some(batch) = self
                    .active_batch
                    .and_then(|id| self.workspace.project.batch(id).ok())
                {
                    ui.label(RichText::new(&batch.name).size(11.0).strong().color(ACCENT));
                }
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.film_filter, FilmFilter::All, "All");
                    ui.selectable_value(&mut self.film_filter, FilmFilter::Keeps, "Keeps");
                    ui.selectable_value(&mut self.film_filter, FilmFilter::Rejects, "Rejects");
                });
                ui.separator();
                let mut photos: Vec<_> = self
                    .workspace
                    .project
                    .photos
                    .iter()
                    .enumerate()
                    .filter(|(_, photo)| {
                        self.active_batch
                            .is_none_or(|batch_id| photo.batch_id == Some(batch_id))
                    })
                    .filter(|(_, photo)| match self.film_filter {
                        FilmFilter::All => true,
                        FilmFilter::Keeps => photo.pick == PickState::Keep,
                        FilmFilter::Rejects => photo.pick == PickState::Reject,
                    })
                    .map(|(index, photo)| (index, photo.id, photo.name.clone(), photo.pick))
                    .collect();
                if self.film_filter == FilmFilter::All {
                    photos.sort_by_key(|(index, _, _, pick)| {
                        (if *pick == PickState::Keep { 0 } else { 1 }, *index)
                    });
                }
                if photos.is_empty() {
                    ui.label(RichText::new("No photos in this view").color(Color32::GRAY));
                    return;
                }
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for (project_index, id, name, pick) in photos {
                            self.ensure_thumbnail(&context, id);
                            let selected = self.selected_ids.contains(&id);
                            let active = self.workspace.project.selected == Some(id);
                            let frame = egui::Frame::new()
                                .fill(if pick == PickState::Keep {
                                    Color32::from_rgb(31, 48, 43)
                                } else if pick == PickState::Reject {
                                    Color32::from_rgb(45, 31, 34)
                                } else if selected {
                                    Color32::from_rgb(50, 41, 67)
                                } else {
                                    SURFACE
                                })
                                .stroke(if active {
                                    Stroke::new(2.0, ACCENT)
                                } else if selected {
                                    Stroke::new(1.5, ACCENT_COOL)
                                } else {
                                    Stroke::new(1.0, Color32::from_gray(50))
                                })
                                .corner_radius(5.0)
                                .inner_margin(6);
                            let inner = frame.show(ui, |ui| {
                                let width = ui.available_width();
                                ui.set_min_width(width);
                                ui.vertical_centered(|ui| {
                                    if let Some(texture) = self.thumbnails.get(&id) {
                                        ui.add(egui::Image::new(texture).fit_to_exact_size(
                                            fit_size(texture.size_vec2(), Vec2::new(width, 108.0)),
                                        ));
                                    } else {
                                        ui.allocate_space(Vec2::new(width, 84.0));
                                    }
                                    ui.label(RichText::new(shorten(&name, 18)).size(11.0));
                                });
                            });
                            if inner.response.interact(Sense::click()).clicked() {
                                let modifiers = ui.input(|input| input.modifiers);
                                self.select_in_filmstrip(id, project_index, modifiers);
                            }
                            ui.add_space(5.0);
                        }
                    });
            });
    }
}

fn toggled_pick(current: PickState, requested: PickState) -> PickState {
    if current == requested {
        PickState::Unmarked
    } else {
        requested
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keep_and_reject_controls_toggle_without_redundant_glyph_buttons() {
        assert_eq!(
            toggled_pick(PickState::Keep, PickState::Keep),
            PickState::Unmarked
        );
        assert_eq!(
            toggled_pick(PickState::Reject, PickState::Reject),
            PickState::Unmarked
        );
        assert_eq!(
            toggled_pick(PickState::Reject, PickState::Keep),
            PickState::Keep
        );
    }
}
