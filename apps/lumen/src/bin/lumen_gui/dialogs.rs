use super::*;

impl LumenApp {
    pub(super) fn status_bar(&mut self, root: &mut egui::Ui) {
        egui::Panel::bottom("status")
            .exact_size(28.0)
            .frame(
                egui::Frame::new()
                    .fill(Color32::from_rgb(19, 20, 23))
                    .inner_margin(6),
            )
            .show(root, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&self.status).size(12.0).color(if self.error {
                        Color32::from_rgb(244, 122, 110)
                    } else {
                        Color32::LIGHT_GRAY
                    }));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if let Some(photo) = self.workspace.project.selected_photo() {
                            ui.label(
                                RichText::new(format!(
                                    "{} x {}  |  {}",
                                    photo.width,
                                    photo.height,
                                    photo.format.to_uppercase()
                                ))
                                .size(12.0)
                                .color(Color32::GRAY),
                            );
                        }
                    });
                });
            });
    }

    pub(super) fn confirmation_window(&mut self, context: &egui::Context) {
        if !self.reset_confirmation {
            return;
        }
        let mut confirm = false;
        let mut cancel = false;
        egui::Window::new("Reset all edits?")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(context, |ui| {
                ui.label(format!(
                    "Reset {} selected photo(s)? This adds a reversible event to each history.",
                    self.selected_photo_ids().len()
                ));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                    if ui
                        .button(
                            RichText::new("Reset all edits")
                                .color(Color32::from_rgb(245, 150, 130)),
                        )
                        .clicked()
                    {
                        confirm = true;
                    }
                });
            });
        if cancel {
            self.reset_confirmation = false;
        }
        if confirm {
            self.reset_confirmation = false;
            let ids = self.selected_photo_ids();
            if !ids.is_empty() && self.execute_and_autosave(Command::Reset { ids: ids.clone() }) {
                for id in ids {
                    self.thumbnails.remove(&id);
                }
                self.draft_id = None;
                self.sync_draft();
            }
        }
    }

    pub(super) fn remove_confirmation_window(&mut self, context: &egui::Context) {
        if !self.remove_confirmation {
            return;
        }
        let ids = self.selected_photo_ids();
        let mut confirm = false;
        let mut cancel = false;
        egui::Window::new("Remove from catalog?")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(context, |ui| {
                ui.label(format!(
                    "Remove {} selected photo(s) from this catalog?",
                    ids.len()
                ));
                ui.label(
                    RichText::new("Original photo files will not be deleted.")
                        .strong()
                        .color(ACCENT),
                );
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                    if ui
                        .button(
                            RichText::new("Remove Photos").color(Color32::from_rgb(245, 150, 130)),
                        )
                        .clicked()
                    {
                        confirm = true;
                    }
                });
            });
        if cancel {
            self.remove_confirmation = false;
        }
        if confirm {
            self.remove_confirmation = false;
            let active_batch = self.active_batch;
            if !ids.is_empty() && self.execute_and_autosave(Command::Remove { ids }) {
                let active_batch =
                    active_batch.filter(|batch_id| self.workspace.project.batch(*batch_id).is_ok());
                self.reset_catalog_view(active_batch.is_none());
                self.active_batch = active_batch;
            }
        }
    }

    pub(super) fn catalog_switch_confirmation_window(&mut self, context: &egui::Context) {
        let Some(action) = self.pending_catalog_switch.clone() else {
            return;
        };
        let mut confirm = false;
        let mut cancel = false;
        egui::Window::new("Leave unsaved catalog?")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(context, |ui| {
                ui.label(format!(
                    "This catalog contains {} photo(s) and has not been saved.",
                    self.workspace.project.photos.len()
                ));
                ui.label("Save it first, or continue and leave this catalog behind.");
                ui.label(
                    RichText::new("Original photo files will not be deleted.").color(Color32::GRAY),
                );
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                    if ui.button("Save Current...").clicked() {
                        self.save_catalog(true);
                        if self.workspace.catalog_path.is_some() {
                            confirm = true;
                        }
                    }
                    if ui
                        .button(
                            RichText::new("Continue Without Saving")
                                .color(Color32::from_rgb(245, 150, 130)),
                        )
                        .clicked()
                    {
                        confirm = true;
                    }
                });
            });
        if cancel {
            self.pending_catalog_switch = None;
        }
        if confirm {
            self.pending_catalog_switch = None;
            self.apply_catalog_switch(action);
        }
    }

    pub(super) fn rename_batch_window(&mut self, context: &egui::Context) {
        if self.rename_batch.is_none() {
            return;
        }
        let mut save = false;
        let mut cancel = false;
        egui::Window::new("Rename Shoot")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(context, |ui| {
                if let Some((_, name)) = self.rename_batch.as_mut() {
                    ui.label("Shoot name");
                    let response = ui.text_edit_singleline(name);
                    if response.lost_focus()
                        && ui.input(|input| input.key_pressed(egui::Key::Enter))
                    {
                        save = true;
                    }
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                    if ui
                        .button(RichText::new("Save Name").color(ACCENT))
                        .clicked()
                    {
                        save = true;
                    }
                });
            });
        if cancel {
            self.rename_batch = None;
        }
        if save
            && let Some((id, name)) = self.rename_batch.take()
            && !self.execute_and_autosave(Command::RenameBatch {
                id,
                name: name.clone(),
            })
        {
            self.rename_batch = Some((id, name));
        }
    }

    pub(super) fn export_window(&mut self, context: &egui::Context) {
        if !self.export_open {
            return;
        }
        let ids = self.selected_photo_ids();
        let mut close = false;
        let mut export = false;
        egui::Window::new(format!("Export {} Photo(s)", ids.len()))
            .collapsible(false)
            .resizable(false)
            .default_width(440.0)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(context, |ui| {
                ui.label(
                    RichText::new("FILE TYPE")
                        .strong()
                        .size(11.0)
                        .color(Color32::GRAY),
                );
                ui.horizontal(|ui| {
                    for (format, label) in [
                        (ExportFormat::Jpeg, "JPEG"),
                        (ExportFormat::Png, "PNG"),
                        (ExportFormat::Tiff, "TIFF"),
                        (ExportFormat::Webp, "WebP"),
                    ] {
                        ui.selectable_value(&mut self.export_format, format, label);
                    }
                });
                if self.export_format == ExportFormat::Jpeg {
                    ui.add(
                        egui::Slider::new(&mut self.export_quality, 1..=100).text("JPEG Quality"),
                    );
                } else if self.export_format == ExportFormat::Webp {
                    ui.label(
                        RichText::new("WebP export is lossless in the current encoder.")
                            .size(10.0)
                            .color(Color32::GRAY),
                    );
                }
                ui.add(
                    egui::Slider::new(&mut self.export_max_size, 0..=8000)
                        .text("Maximum long edge")
                        .suffix(" px"),
                );
                if self.export_max_size == 0 {
                    ui.label(
                        RichText::new("0 uses full resolution")
                            .size(10.0)
                            .color(Color32::GRAY),
                    );
                }
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Choose Export Folder...").clicked() {
                        self.export_directory = rfd::FileDialog::new().pick_folder();
                    }
                    if let Some(directory) = &self.export_directory {
                        ui.label(shorten(&directory.display().to_string(), 38));
                    } else {
                        ui.label(RichText::new("No folder selected").color(Color32::GRAY));
                    }
                });
                let estimate = estimate_export_bytes(
                    &self.workspace,
                    &ids,
                    self.export_format,
                    self.export_quality,
                    self.export_max_size,
                );
                ui.add_space(4.0);
                ui.label(format!(
                    "Estimated output: {} total | about {} per photo",
                    format_bytes(estimate),
                    format_bytes(estimate / ids.len().max(1) as u64)
                ));
                ui.label(
                    RichText::new("Estimate varies with image detail and compression.")
                        .size(10.0)
                        .color(Color32::GRAY),
                );
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        close = true;
                    }
                    if ui
                        .add_enabled(
                            self.export_directory.is_some() && !ids.is_empty(),
                            egui::Button::new(RichText::new("Export").color(ACCENT)),
                        )
                        .clicked()
                    {
                        export = true;
                    }
                });
            });
        if close {
            self.export_open = false;
        }
        if export
            && let Some(directory) = self.export_directory.clone()
            && self.execute(Command::ExportBatch {
                ids,
                directory,
                format: self.export_format,
                max_size: (self.export_max_size > 0).then_some(self.export_max_size),
                quality: self.export_quality,
            })
        {
            self.export_open = false;
        }
    }
}
