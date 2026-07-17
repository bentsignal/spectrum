use super::*;

impl LumenApp {
    pub(super) fn open_batch(&mut self, batch_id: u64) {
        let first = self
            .workspace
            .project
            .photos
            .iter()
            .find(|photo| photo.batch_id == Some(batch_id))
            .map(|photo| photo.id);
        self.active_batch = Some(batch_id);
        self.library_mode = false;
        self.film_filter = FilmFilter::All;
        if let Some(id) = first {
            self.select(id);
        }
    }

    pub(super) fn library_canvas(&mut self, root: &mut egui::Ui) {
        let context = root.ctx().clone();
        let mut batches: Vec<_> = self
            .workspace
            .project
            .batches
            .iter()
            .map(|batch| {
                let photos = self
                    .workspace
                    .project
                    .photos
                    .iter()
                    .filter(|photo| photo.batch_id == Some(batch.id))
                    .map(|photo| (photo.id, photo.name.clone(), photo.pick))
                    .collect::<Vec<_>>();
                (
                    batch.id,
                    batch.name.clone(),
                    batch.captured_date.clone(),
                    batch.captured_end_date.clone(),
                    batch.imported_date.clone(),
                    photos,
                )
            })
            .filter(|(_, _, _, _, _, photos)| !photos.is_empty())
            .collect();
        batches.sort_by_key(|(id, _, start, _, imported, _)| {
            (start.clone().unwrap_or_else(|| imported.clone()), *id)
        });

        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(CANVAS).inner_margin(24))
            .show(root, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new(&self.workspace.project.name)
                                .size(24.0)
                                .color(Color32::from_gray(230)),
                        );
                        ui.label(
                            RichText::new(format!(
                                "{} shoots  |  {} photos",
                                batches.len(),
                                self.workspace.project.photos.len()
                            ))
                            .size(12.0)
                            .color(Color32::GRAY),
                        );
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Import a Shoot").clicked() {
                            self.import_dialog();
                        }
                    });
                });
                ui.add_space(12.0);

                if batches.is_empty() {
                    let available = ui.available_rect_before_wrap();
                    ui.allocate_rect(available, Sense::hover());
                    let content = Rect::from_center_size(
                        available.center(),
                        Vec2::new(available.width().min(520.0), 150.0),
                    );
                    ui.scope_builder(
                        egui::UiBuilder::new()
                            .max_rect(content)
                            .layout(egui::Layout::top_down(egui::Align::Center)),
                        |ui| {
                            ui.label(
                                RichText::new("Your timeline starts here")
                                    .size(24.0)
                                    .color(Color32::from_gray(225)),
                            );
                            ui.add_space(8.0);
                            ui.label(
                                RichText::new(
                                    "Import a shoot and Lumen will place it on the timeline.",
                                )
                                .size(13.0)
                                .color(Color32::GRAY),
                            );
                            ui.add_space(14.0);
                            if ui.button("Choose Photos").clicked() {
                                self.import_dialog();
                            }
                        },
                    );
                    return;
                }

                let column_height = ui.available_height().max(280.0);
                let mut open_batch = None;
                let mut rename_batch = None;
                egui::ScrollArea::horizontal()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.horizontal_top(|ui| {
                            for (batch_id, name, start, end, imported, photos) in &batches {
                                for (photo_id, _, _) in photos {
                                    self.ensure_thumbnail(&context, *photo_id);
                                }
                                ui.allocate_ui_with_layout(
                                    Vec2::new(210.0, column_height),
                                    egui::Layout::top_down(egui::Align::Center),
                                    |ui| {
                                        ui.label(
                                            RichText::new(shoot_date_label(
                                                start.as_deref(),
                                                end.as_deref(),
                                                imported,
                                            ))
                                            .size(11.0)
                                            .strong()
                                            .color(Color32::from_gray(165)),
                                        );
                                        ui.add_space(5.0);
                                        egui::Frame::new()
                                            .fill(SURFACE)
                                            .stroke(Stroke::new(1.0, Color32::from_gray(54)))
                                            .corner_radius(8.0)
                                            .inner_margin(10)
                                            .show(ui, |ui| {
                                                ui.vertical(|ui| {
                                                    ui.set_width(190.0);
                                                    ui.set_height(column_height - 48.0);
                                                    if ui
                                                        .add_sized(
                                                            [190.0, 28.0],
                                                            egui::Button::new(
                                                                RichText::new(name)
                                                                    .strong()
                                                                    .color(ACCENT),
                                                            ),
                                                        )
                                                        .on_hover_text("Open this shoot")
                                                        .clicked()
                                                    {
                                                        open_batch = Some(*batch_id);
                                                    }
                                                    ui.horizontal(|ui| {
                                                        ui.label(
                                                            RichText::new(format!(
                                                                "{} photo{}",
                                                                photos.len(),
                                                                if photos.len() == 1 {
                                                                    ""
                                                                } else {
                                                                    "s"
                                                                }
                                                            ))
                                                            .size(10.0)
                                                            .color(Color32::GRAY),
                                                        );
                                                        ui.with_layout(
                                                            egui::Layout::right_to_left(
                                                                egui::Align::Center,
                                                            ),
                                                            |ui| {
                                                                if ui
                                                                    .small_button("Rename")
                                                                    .clicked()
                                                                {
                                                                    rename_batch = Some((
                                                                        *batch_id,
                                                                        name.clone(),
                                                                    ));
                                                                }
                                                            },
                                                        );
                                                    });
                                                    ui.separator();
                                                    egui::ScrollArea::vertical()
                                                        .id_salt(("batch-column", batch_id))
                                                        .max_height(
                                                            (column_height - 124.0).max(160.0),
                                                        )
                                                        .auto_shrink([false, false])
                                                        .show(ui, |ui| {
                                                            ui.set_width(190.0);
                                                            for (photo_id, photo_name, pick) in
                                                                photos
                                                            {
                                                                ui.vertical_centered(|ui| {
                                                                    if let Some(texture) = self
                                                                        .thumbnails
                                                                        .get(photo_id)
                                                                    {
                                                                        let size = fit_size(
                                                                            texture.size_vec2(),
                                                                            Vec2::new(180.0, 112.0),
                                                                        );
                                                                        if ui
                                                                            .add(
                                                                                egui::Image::new(
                                                                                    texture,
                                                                                )
                                                                                .fit_to_exact_size(
                                                                                    size,
                                                                                )
                                                                                .sense(
                                                                                    Sense::click(),
                                                                                ),
                                                                            )
                                                                            .on_hover_text(
                                                                                "Open this shoot",
                                                                            )
                                                                            .clicked()
                                                                        {
                                                                            open_batch =
                                                                                Some(*batch_id);
                                                                        }
                                                                    }
                                                                    ui.label(
                                                                        RichText::new(shorten(
                                                                            photo_name, 24,
                                                                        ))
                                                                        .size(10.0),
                                                                    );
                                                                    match pick {
                                                                        PickState::Keep => {
                                                                            ui.label(
                                                                                RichText::new("+")
                                                                                    .strong()
                                                                                    .color(ACCENT),
                                                                            );
                                                                        }
                                                                        PickState::Reject => {
                                                                            ui.label(
                                                                            RichText::new("x")
                                                                                .color(
                                                                                Color32::from_rgb(
                                                                                    225, 112, 120,
                                                                                ),
                                                                            ),
                                                                        );
                                                                        }
                                                                        PickState::Unmarked => {}
                                                                    }
                                                                    ui.add_space(8.0);
                                                                });
                                                            }
                                                        });
                                                });
                                            });
                                    },
                                );
                                ui.add_space(8.0);
                            }
                        });
                    });
                if let Some(batch_id) = open_batch {
                    self.open_batch(batch_id);
                }
                if let Some(batch) = rename_batch {
                    self.rename_batch = Some(batch);
                }
            });
    }
}
