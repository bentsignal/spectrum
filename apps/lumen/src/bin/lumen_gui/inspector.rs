use super::*;

impl LumenApp {
    pub(super) fn inspector(&mut self, root: &mut egui::Ui) {
        egui::Panel::right("inspector")
            .resizable(true)
            .default_size(330.0)
            .size_range(300.0..=430.0)
            .frame(
                egui::Frame::new()
                    .fill(PANEL)
                    .stroke(Stroke::new(1.0, Color32::from_gray(48)))
                    .inner_margin(egui::Margin::same(14)),
            )
            .show(root, |ui| {
                let Some(id) = self.workspace.project.selected else {
                    ui.heading("Develop");
                    ui.label(RichText::new("Select a photo to edit").color(Color32::GRAY));
                    return;
                };
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        self.history_ui(ui, id);
                        ui.add_space(4.0);
                        self.histogram_ui(ui);
                        self.photo_details_ui(ui, id);
                        ui.add_space(4.0);
                        if !self.workspace.is_durable() {
                            ui.heading("Develop");
                            ui.add_space(3.0);
                        }
                        let mut draft = self.draft.clone();
                        let mut changed = false;
                        let mut commit = false;
                        egui::CollapsingHeader::new("Light")
                            .default_open(true)
                            .show(ui, |ui| {
                                slider(
                                    ui,
                                    "Exposure",
                                    &mut draft.exposure,
                                    -5.0..=5.0,
                                    &mut changed,
                                    &mut commit,
                                );
                                slider(
                                    ui,
                                    "Contrast",
                                    &mut draft.contrast,
                                    -100.0..=100.0,
                                    &mut changed,
                                    &mut commit,
                                );
                                slider(
                                    ui,
                                    "Highlights",
                                    &mut draft.highlights,
                                    -100.0..=100.0,
                                    &mut changed,
                                    &mut commit,
                                );
                                slider(
                                    ui,
                                    "Shadows",
                                    &mut draft.shadows,
                                    -100.0..=100.0,
                                    &mut changed,
                                    &mut commit,
                                );
                                slider(
                                    ui,
                                    "Whites",
                                    &mut draft.whites,
                                    -100.0..=100.0,
                                    &mut changed,
                                    &mut commit,
                                );
                                slider(
                                    ui,
                                    "Blacks",
                                    &mut draft.blacks,
                                    -100.0..=100.0,
                                    &mut changed,
                                    &mut commit,
                                );
                            });
                        egui::CollapsingHeader::new("Color")
                            .default_open(true)
                            .show(ui, |ui| {
                                slider(
                                    ui,
                                    "Temperature",
                                    &mut draft.temperature,
                                    -100.0..=100.0,
                                    &mut changed,
                                    &mut commit,
                                );
                                slider(
                                    ui,
                                    "Tint",
                                    &mut draft.tint,
                                    -100.0..=100.0,
                                    &mut changed,
                                    &mut commit,
                                );
                                slider(
                                    ui,
                                    "Vibrance",
                                    &mut draft.vibrance,
                                    -100.0..=100.0,
                                    &mut changed,
                                    &mut commit,
                                );
                                slider(
                                    ui,
                                    "Saturation",
                                    &mut draft.saturation,
                                    -100.0..=100.0,
                                    &mut changed,
                                    &mut commit,
                                );
                            });
                        egui::CollapsingHeader::new("Color Grading")
                            .default_open(false)
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    for (index, label) in
                                        ["Shadows", "Midtones", "Highlights"].into_iter().enumerate()
                                    {
                                        if ui
                                            .selectable_label(self.grade_range == index, label)
                                            .clicked()
                                        {
                                            self.grade_range = index;
                                        }
                                    }
                                });
                                {
                                    let grade = match self.grade_range {
                                        0 => &mut draft.color_grading.shadows,
                                        1 => &mut draft.color_grading.midtones,
                                        _ => &mut draft.color_grading.highlights,
                                    };
                                    ui.horizontal(|ui| {
                                        grade_swatch(ui, *grade);
                                        ui.label(
                                            RichText::new("Tonal tint")
                                                .size(10.0)
                                                .color(Color32::GRAY),
                                        );
                                        if ui.small_button("Reset range").clicked() {
                                            *grade = ColorGrade::default();
                                            changed = true;
                                            commit = true;
                                        }
                                    });
                                    slider(
                                        ui,
                                        "Hue",
                                        &mut grade.hue,
                                        0.0..=360.0,
                                        &mut changed,
                                        &mut commit,
                                    );
                                    slider(
                                        ui,
                                        "Saturation",
                                        &mut grade.saturation,
                                        0.0..=100.0,
                                        &mut changed,
                                        &mut commit,
                                    );
                                    slider(
                                        ui,
                                        "Luminance",
                                        &mut grade.luminance,
                                        -100.0..=100.0,
                                        &mut changed,
                                        &mut commit,
                                    );
                                }
                                slider(
                                    ui,
                                    "Balance",
                                    &mut draft.color_grading.balance,
                                    -100.0..=100.0,
                                    &mut changed,
                                    &mut commit,
                                );
                            });
                        egui::CollapsingHeader::new("Presence & Detail")
                            .default_open(true)
                            .show(ui, |ui| {
                                slider(
                                    ui,
                                    "Texture",
                                    &mut draft.texture,
                                    -100.0..=100.0,
                                    &mut changed,
                                    &mut commit,
                                );
                                slider(
                                    ui,
                                    "Clarity",
                                    &mut draft.clarity,
                                    -100.0..=100.0,
                                    &mut changed,
                                    &mut commit,
                                );
                                slider(
                                    ui,
                                    "Dehaze",
                                    &mut draft.dehaze,
                                    -100.0..=100.0,
                                    &mut changed,
                                    &mut commit,
                                );
                                slider(
                                    ui,
                                    "Sharpening",
                                    &mut draft.sharpening,
                                    0.0..=100.0,
                                    &mut changed,
                                    &mut commit,
                                );
                                slider(
                                    ui,
                                    "Noise Reduction",
                                    &mut draft.noise_reduction,
                                    0.0..=100.0,
                                    &mut changed,
                                    &mut commit,
                                );
                                slider(
                                    ui,
                                    "Vignette",
                                    &mut draft.vignette,
                                    -100.0..=100.0,
                                    &mut changed,
                                    &mut commit,
                                );
                            });
                        egui::CollapsingHeader::new("Crop & Transform")
                            .default_open(true)
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
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
                                    } else if ui.button("Edit Crop on Image").clicked() {
                                        self.begin_crop();
                                    }
                                    if draft.crop.is_some()
                                        && ui.small_button("Clear").clicked()
                                    {
                                        draft.crop = None;
                                        changed = true;
                                        commit = true;
                                    }
                                });
                                if self.crop_mode {
                                    ui.label(
                                        RichText::new(
                                            "Drag corners, edges, or the crop interior. The rule-of-thirds overlay updates live.",
                                        )
                                        .size(10.0)
                                        .color(Color32::GRAY),
                                    );
                                    let source_aspect = self
                                        .workspace
                                        .project
                                        .photo(id)
                                        .map(|photo| {
                                            let aspect = photo.width as f32 / photo.height.max(1) as f32;
                                            if matches!(draft.rotation, 90 | 270) { 1.0 / aspect } else { aspect }
                                        })
                                        .unwrap_or(1.0);
                                    ui.horizontal(|ui| {
                                        ui.label("Aspect");
                                        for (label, ratio) in
                                            [("Free", None), ("1:1", Some(1.0)), ("4:5", Some(0.8)), ("16:9", Some(16.0 / 9.0))]
                                        {
                                            if ui.small_button(label).clicked()
                                                && let Some(ratio) = ratio
                                            {
                                                set_crop_aspect(&mut self.crop_draft, ratio, source_aspect);
                                            }
                                        }
                                    });
                                } else {
                                    ui.label(
                                        RichText::new(if draft.crop.is_some() {
                                            "A nondestructive crop is active"
                                        } else {
                                            "No crop applied"
                                        })
                                        .size(10.0)
                                        .color(Color32::GRAY),
                                    );
                                }
                                slider(
                                    ui,
                                    "Straighten",
                                    &mut draft.straighten,
                                    -45.0..=45.0,
                                    &mut changed,
                                    &mut commit,
                                );
                                ui.horizontal(|ui| {
                                    if ui.button("90 CCW").clicked() {
                                        draft.rotation = (draft.rotation - 90).rem_euclid(360);
                                        changed = true;
                                        commit = true;
                                    }
                                    if ui.button("90 CW").clicked() {
                                        draft.rotation = (draft.rotation + 90).rem_euclid(360);
                                        changed = true;
                                        commit = true;
                                    }
                                    if ui.button("Flip H").clicked() {
                                        draft.flip_horizontal = !draft.flip_horizontal;
                                        changed = true;
                                        commit = true;
                                    }
                                });
                            });
                        egui::CollapsingHeader::new("Dust & Spot Removal")
                            .default_open(false)
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    if ui
                                        .selectable_label(self.spot_mode, "Repair Brush")
                                        .on_hover_text("Paint over dust or small smudges on the photo")
                                        .clicked()
                                    {
                                        self.spot_mode = !self.spot_mode;
                                        self.crop_mode = false;
                                        self.compare_mode = CompareMode::Edited;
                                    }
                                    if ui
                                        .add_enabled(!draft.spots.is_empty(), egui::Button::new("Undo Dab"))
                                        .clicked()
                                    {
                                        draft.spots.pop();
                                        changed = true;
                                        commit = true;
                                    }
                                    if ui
                                        .add_enabled(!draft.spots.is_empty(), egui::Button::new("Clear"))
                                        .clicked()
                                    {
                                        draft.spots.clear();
                                        changed = true;
                                        commit = true;
                                    }
                                });
                                ui.add(
                                    egui::Slider::new(&mut self.spot_radius, 0.005..=0.12)
                                        .text("Brush size")
                                        .custom_formatter(|value, _| format!("{:.0}%", value * 100.0)),
                                );
                                ui.label(
                                    RichText::new(format!(
                                        "{} repair dab(s) | drag on the image to paint",
                                        draft.spots.len()
                                    ))
                                    .size(10.0)
                                    .color(Color32::GRAY),
                                );
                            });
                        egui::CollapsingHeader::new("Color Mixer (HSL)")
                            .default_open(false)
                            .show(ui, |ui| {
                                ui.horizontal_wrapped(|ui| {
                                    for index in 0..8 {
                                        if ui
                                            .selectable_label(
                                                self.hsl_band == index,
                                                RichText::new(HSL_NAMES[index])
                                                    .color(HSL_COLORS[index]),
                                            )
                                            .clicked()
                                        {
                                            self.hsl_band = index;
                                        }
                                    }
                                });
                                let band = draft.hsl.band_mut(self.hsl_band);
                                slider(
                                    ui,
                                    "Hue",
                                    &mut band.hue,
                                    -100.0..=100.0,
                                    &mut changed,
                                    &mut commit,
                                );
                                slider(
                                    ui,
                                    "Saturation",
                                    &mut band.saturation,
                                    -100.0..=100.0,
                                    &mut changed,
                                    &mut commit,
                                );
                                slider(
                                    ui,
                                    "Luminance",
                                    &mut band.luminance,
                                    -100.0..=100.0,
                                    &mut changed,
                                    &mut commit,
                                );
                            });
                        egui::CollapsingHeader::new("Tone Curve")
                            .default_open(true)
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    for (index, name) in
                                        ["Master", "Red", "Green", "Blue"].into_iter().enumerate()
                                    {
                                        if ui
                                            .selectable_label(self.curve_channel == index, name)
                                            .clicked()
                                        {
                                            self.curve_channel = index;
                                        }
                                    }
                                });
                                let curve = match self.curve_channel {
                                    0 => &mut draft.curves.master,
                                    1 => &mut draft.curves.red,
                                    2 => &mut draft.curves.green,
                                    _ => &mut draft.curves.blue,
                                };
                                let (curve_changed, curve_commit) =
                                    tone_curve_editor(ui, curve, self.curve_channel);
                                changed |= curve_changed;
                                commit |= curve_commit;
                                if ui.small_button("Reset this curve").clicked() {
                                    *curve = ToneCurve::default();
                                    changed = true;
                                    commit = true;
                                }
                            });
                        if changed {
                            self.draft = draft.sanitized();
                            self.preview = None;
                        }
                        if commit {
                            self.finish_edit(id);
                        }
                        ui.separator();
                        ui.horizontal(|ui| {
                            if ui.button("Copy Edits").clicked() {
                                self.execute(Command::CopyEdits { id });
                            }
                            if ui
                                .add_enabled(
                                    self.workspace.clipboard.is_some(),
                                    egui::Button::new("Paste Edits"),
                                )
                                .clicked()
                                && self.execute_and_commit(Command::PasteEdits {
                                    ids: self.selected_photo_ids(),
                                })
                            {
                                for selected in self.selected_photo_ids() {
                                    self.thumbnails.remove(&selected);
                                }
                                self.draft_id = None;
                                self.sync_draft();
                            }
                            if ui
                                .button(
                                    RichText::new("Reset...").color(Color32::from_rgb(245, 150, 130)),
                                )
                                .clicked()
                            {
                                self.reset_confirmation = true;
                            }
                        });
                        ui.separator();
                        self.presets_ui(ui, id);
                    });
            });
    }

    pub(super) fn histogram_ui(&self, ui: &mut egui::Ui) {
        egui::Frame::new()
            .fill(CANVAS)
            .stroke(Stroke::new(1.0, Color32::from_gray(55)))
            .corner_radius(7.0)
            .inner_margin(8)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("HISTOGRAM")
                            .strong()
                            .size(10.0)
                            .color(Color32::GRAY),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new("RGB + luminance")
                                .size(9.0)
                                .color(Color32::from_gray(105)),
                        );
                    });
                });
                let (rect, _) =
                    ui.allocate_exact_size(Vec2::new(ui.available_width(), 112.0), Sense::hover());
                if let Some(histogram) = &self.histogram {
                    paint_histogram(ui, rect, histogram);
                }
            });
    }

    pub(super) fn photo_details_ui(&self, ui: &mut egui::Ui, id: u64) {
        egui::CollapsingHeader::new("Photo Details")
            .default_open(true)
            .show(ui, |ui| {
                let Ok(photo) = self.workspace.project.photo(id) else {
                    return;
                };
                let metadata = &photo.metadata;
                detail_row(
                    ui,
                    "Camera",
                    metadata
                        .camera_model
                        .as_deref()
                        .or(metadata.camera_make.as_deref())
                        .unwrap_or("--"),
                );
                detail_row(ui, "Lens", metadata.lens.as_deref().unwrap_or("--"));
                detail_row(
                    ui,
                    "Capture",
                    &format!(
                        "{}  |  {}  |  {}  |  {}",
                        metadata
                            .iso
                            .map_or_else(|| "ISO --".into(), |iso| format!("ISO {iso}")),
                        metadata
                            .focal_length_mm
                            .map_or_else(|| "-- mm".into(), |value| format!("{value:.0} mm")),
                        metadata
                            .aperture
                            .map_or_else(|| "f/--".into(), |value| format!("f/{value:.1}")),
                        format_shutter(metadata.shutter_seconds),
                    ),
                );
                if let Some(captured) = &metadata.captured_at {
                    detail_row(ui, "Date", captured);
                }
            });
    }

    pub(super) fn presets_ui(&mut self, ui: &mut egui::Ui, id: u64) {
        egui::CollapsingHeader::new("Presets")
            .default_open(false)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.preset_name)
                            .hint_text("Preset name")
                            .desired_width(150.0),
                    );
                    if ui
                        .add_enabled(
                            !self.preset_name.trim().is_empty(),
                            egui::Button::new("Save Current"),
                        )
                        .clicked()
                        && self.execute_and_commit(Command::SavePreset {
                            name: self.preset_name.trim().to_owned(),
                            from_id: id,
                        })
                    {
                        self.preset_name.clear();
                    }
                });
                ui.label(
                    RichText::new("Presets save development settings, not crop or rotation.")
                        .size(9.0)
                        .color(Color32::GRAY),
                );
                let presets: Vec<_> = self
                    .workspace
                    .project
                    .presets
                    .iter()
                    .map(|preset| (preset.id, preset.name.clone()))
                    .collect();
                for (preset_id, name) in presets {
                    ui.horizontal(|ui| {
                        if ui.button(&name).clicked()
                            && self.execute_and_commit(Command::ApplyPreset {
                                preset_id,
                                ids: self.selected_photo_ids(),
                            })
                        {
                            for selected in self.selected_photo_ids() {
                                self.thumbnails.remove(&selected);
                            }
                            self.draft_id = None;
                            self.sync_draft();
                        }
                        if ui
                            .small_button("x")
                            .on_hover_text("Delete preset")
                            .clicked()
                        {
                            self.execute_and_commit(Command::DeletePreset { id: preset_id });
                        }
                    });
                }
                if self.workspace.project.presets.is_empty() {
                    ui.label(RichText::new("No saved presets yet").color(Color32::GRAY));
                }
            });
    }

    pub(super) fn history_ui(&mut self, ui: &mut egui::Ui, id: u64) {
        if self.workspace.is_durable() {
            ui.horizontal(|ui| {
                ui.heading("Develop");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .button(format!("History  {}", history_shortcut_label()))
                        .clicked()
                    {
                        self.toggle_history();
                    }
                });
            });
            return;
        }
        egui::CollapsingHeader::new("Edit History")
            .default_open(false)
            .show(ui, |ui| {
                let Some(photo) = self.workspace.project.photo(id).ok() else {
                    return;
                };
                let cursor = photo.history_cursor;
                let entries: Vec<_> = photo
                    .history
                    .iter()
                    .enumerate()
                    .map(|(index, entry)| (index, entry.label.clone()))
                    .collect();
                for (index, label) in entries.into_iter().rev().take(20) {
                    let marker = if index == cursor { ">" } else { " " };
                    if ui
                        .selectable_label(index == cursor, format!("{marker}  {label}"))
                        .clicked()
                        && self.execute_and_commit(Command::HistoryJump { id, index })
                    {
                        self.draft_id = None;
                        self.sync_draft();
                    }
                }
            });
    }
}
