use super::*;

impl LumenApp {
    pub(super) fn show_catalog(&mut self) {
        self.history_open = false;
        self.hide_terminal();
        self.library_mode = true;
    }

    pub(super) fn show_editor(&mut self) {
        self.history_open = false;
        self.hide_terminal();
        self.return_to_photo_view();
    }

    pub(super) fn return_to_photo_view(&mut self) {
        if self.active_batch.is_none() {
            self.active_batch = self
                .workspace
                .project
                .selected_photo()
                .and_then(|photo| photo.batch_id);
        }
        if self.workspace.project.selected.is_none() {
            let first = self
                .workspace
                .project
                .photos
                .iter()
                .find(|photo| {
                    self.active_batch
                        .is_none_or(|batch_id| photo.batch_id == Some(batch_id))
                })
                .map(|photo| photo.id);
            if let Some(id) = first {
                self.select(id);
            }
        }
        self.library_mode = false;
    }

    pub(super) fn open_batch(&mut self, batch_id: u64, requested_photo: Option<u64>) {
        let selected = editor_photo_for_batch(&self.workspace.project, batch_id, requested_photo);
        self.active_batch = Some(batch_id);
        self.library_mode = false;
        self.film_filter = FilmFilter::All;
        if let Some(id) = selected {
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
                    ui.allocate_ui_with_layout(
                        Vec2::new(ui.available_width(), 44.0),
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
                            if ui
                                .button("Import Photos")
                                .on_hover_text(format!(
                                    "Import photos as a new shoot  {}",
                                    import_photos_shortcut_label()
                                ))
                                .clicked()
                            {
                                self.import_dialog();
                            }
                            if ui
                                .button("New Shoot")
                                .on_hover_text(format!(
                                    "Create a new shoot catalog  {}",
                                    new_shoot_shortcut_label()
                                ))
                                .clicked()
                            {
                                self.new_catalog();
                            }
                        },
                    );
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
                                                        open_batch = Some((*batch_id, None));
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
                                                                egui::Frame::new()
                                                                    .fill(match pick {
                                                                        PickState::Keep => {
                                                                            Color32::from_rgb(
                                                                                31, 48, 43,
                                                                            )
                                                                        }
                                                                        PickState::Reject => {
                                                                            Color32::from_rgb(
                                                                                45, 31, 34,
                                                                            )
                                                                        }
                                                                        PickState::Unmarked => {
                                                                            Color32::TRANSPARENT
                                                                        }
                                                                    })
                                                                    .corner_radius(5.0)
                                                                    .inner_margin(4)
                                                                    .show(ui, |ui| {
                                                                        ui.vertical_centered(|ui| {
                                                                            if let Some(texture) =
                                                                                self.thumbnails
                                                                                    .get(photo_id)
                                                                            {
                                                                                let size = fit_size(
                                                                                    texture
                                                                                        .size_vec2(),
                                                                                    Vec2::new(
                                                                                        172.0,
                                                                                        112.0,
                                                                                    ),
                                                                                );
                                                                                if ui
                                                                                    .add(
                                                                                        egui::Image::new(texture)
                                                                                            .fit_to_exact_size(size)
                                                                                            .sense(Sense::click()),
                                                                                    )
                                                                                    .on_hover_text(
                                                                                        "Open this photo in Editor",
                                                                                    )
                                                                                    .clicked()
                                                                                {
                                                                                    open_batch = Some((
                                                                                        *batch_id,
                                                                                        Some(*photo_id),
                                                                                    ));
                                                                                }
                                                                            }
                                                                            ui.label(
                                                                                RichText::new(
                                                                                    shorten(
                                                                                        photo_name,
                                                                                        24,
                                                                                    ),
                                                                                )
                                                                                .size(10.0),
                                                                            );
                                                                        });
                                                                    });
                                                                ui.add_space(8.0);
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
                if let Some((batch_id, photo_id)) = open_batch {
                    self.open_batch(batch_id, photo_id);
                }
                if let Some(batch) = rename_batch {
                    self.rename_batch = Some(batch);
                }
            });
    }
}

fn new_shoot_shortcut_label() -> &'static str {
    if cfg!(target_os = "macos") {
        "⌘N"
    } else {
        "Ctrl+N"
    }
}

fn import_photos_shortcut_label() -> &'static str {
    if cfg!(target_os = "macos") {
        "⌘I"
    } else {
        "Ctrl+I"
    }
}

fn editor_photo_for_batch(
    project: &Project,
    batch_id: u64,
    requested_photo: Option<u64>,
) -> Option<u64> {
    let belongs_to_batch = |id| {
        project
            .photo(id)
            .is_ok_and(|photo| photo.batch_id == Some(batch_id))
    };
    requested_photo
        .filter(|id| belongs_to_batch(*id))
        .or_else(|| project.selected.filter(|id| belongs_to_batch(*id)))
        .or_else(|| {
            project
                .photos
                .iter()
                .find(|photo| photo.batch_id == Some(batch_id))
                .map(|photo| photo.id)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project_with_two_shoots() -> Project {
        serde_json::from_value(serde_json::json!({
            "name": "Navigation",
            "photos": [
                {
                    "id": 10,
                    "name": "first.jpg",
                    "path": "/tmp/first.jpg",
                    "width": 10,
                    "height": 10,
                    "batch_id": 1
                },
                {
                    "id": 11,
                    "name": "clicked.jpg",
                    "path": "/tmp/clicked.jpg",
                    "width": 10,
                    "height": 10,
                    "batch_id": 1
                },
                {
                    "id": 20,
                    "name": "other.jpg",
                    "path": "/tmp/other.jpg",
                    "width": 10,
                    "height": 10,
                    "batch_id": 2
                }
            ],
            "selected": 11,
            "batches": [
                {
                    "id": 1,
                    "name": "Shoot One",
                    "imported_date": "2026-07-23"
                },
                {
                    "id": 2,
                    "name": "Shoot Two",
                    "imported_date": "2026-07-23"
                }
            ]
        }))
        .expect("navigation fixture should deserialize")
    }

    #[test]
    fn thumbnail_navigation_prefers_the_exact_clicked_photo() {
        let project = project_with_two_shoots();
        assert_eq!(editor_photo_for_batch(&project, 1, Some(10)), Some(10));
    }

    #[test]
    fn shoot_navigation_preserves_last_photo_then_falls_back_to_first() {
        let mut project = project_with_two_shoots();
        assert_eq!(editor_photo_for_batch(&project, 1, None), Some(11));

        project.selected = Some(20);
        assert_eq!(editor_photo_for_batch(&project, 1, None), Some(10));
    }

    #[test]
    fn invalid_thumbnail_target_cannot_escape_the_requested_shoot() {
        let mut project = project_with_two_shoots();
        project.selected = Some(20);
        assert_eq!(editor_photo_for_batch(&project, 1, Some(20)), Some(10));
    }

    #[test]
    fn primary_catalog_action_shortcuts_match_the_host_platform() {
        if cfg!(target_os = "macos") {
            assert_eq!(new_shoot_shortcut_label(), "⌘N");
            assert_eq!(import_photos_shortcut_label(), "⌘I");
        } else {
            assert_eq!(new_shoot_shortcut_label(), "Ctrl+N");
            assert_eq!(import_photos_shortcut_label(), "Ctrl+I");
        }
    }
}
