use super::*;

#[derive(Clone)]
enum LayerRowKind {
    Raster(PathBuf),
    Text([u8; 4]),
    Shape([u8; 4]),
}

#[derive(Clone)]
struct LayerRowData {
    id: u64,
    name: String,
    visible: bool,
    locked: bool,
    kind: LayerRowKind,
}

impl From<&Layer> for LayerRowData {
    fn from(layer: &Layer) -> Self {
        let kind = match &layer.kind {
            LayerKind::Raster { path, .. } => LayerRowKind::Raster(path.clone()),
            LayerKind::Text { color, .. } => LayerRowKind::Text(*color),
            LayerKind::Rectangle { color, .. } => LayerRowKind::Shape(*color),
        };
        Self {
            id: layer.id,
            name: layer.name.clone(),
            visible: layer.visible,
            locked: layer.locked,
            kind,
        }
    }
}

impl LayerRowData {
    fn label(&self) -> &'static str {
        match &self.kind {
            LayerRowKind::Raster(_) => "Image",
            LayerRowKind::Text(_) => "Text",
            LayerRowKind::Shape(_) => "Shape",
        }
    }

    fn fill(&self) -> Color32 {
        match &self.kind {
            LayerRowKind::Text(color) | LayerRowKind::Shape(color) => color32(*color),
            LayerRowKind::Raster(_) => RAISED,
        }
    }

    fn raster_path(&self) -> Option<&Path> {
        match &self.kind {
            LayerRowKind::Raster(path) => Some(path),
            _ => None,
        }
    }

    fn matches_query(&self, query: &str) -> bool {
        let query = query.trim().to_ascii_lowercase();
        query.is_empty()
            || self.name.to_ascii_lowercase().contains(&query)
            || self.label().to_ascii_lowercase().contains(&query)
            || format!("#{}", self.id).contains(&query)
    }
}

impl PrismApp {
    pub(super) fn layers_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(RichText::new("COMPOSITION").size(11.0).strong().color(TEXT));
                ui.label(
                    RichText::new(format!(
                        "{} object{} on canvas",
                        self.workspace.document.layers.len(),
                        if self.workspace.document.layers.len() == 1 {
                            ""
                        } else {
                            "s"
                        }
                    ))
                    .size(10.0)
                    .color(MUTED),
                );
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .button(RichText::new("+ Place").strong().color(ACCENT))
                    .on_hover_text("Choose a linked image for the composition")
                    .clicked()
                {
                    self.add_raster();
                }
            });
        });
        ui.add_space(8.0);
        let search = ui.add(
            egui::TextEdit::singleline(&mut self.composition_query)
                .hint_text("Jump to object…   ⌘J")
                .desired_width(f32::INFINITY),
        );
        if self.composition_search_focus {
            search.request_focus();
            self.composition_search_focus = false;
        }
        if search.has_focus() && ui.input(|input| input.key_pressed(egui::Key::Escape)) {
            self.composition_query.clear();
        }
        let layers: Vec<_> = self
            .workspace
            .document
            .layers
            .iter()
            .map(LayerRowData::from)
            .collect();
        let selected_index = self.workspace.document.selected.and_then(|selected| {
            self.workspace
                .document
                .layers
                .iter()
                .position(|layer| layer.id == selected)
        });
        if let (Some(index), Some(layer)) = (
            selected_index,
            self.workspace
                .document
                .selected
                .and_then(|id| layers.iter().find(|layer| layer.id == id)),
        ) {
            ui.add_space(8.0);
            self.focus_card(ui, layer, index, layers.len());
        }
        let filtered: Vec<_> = layers
            .iter()
            .enumerate()
            .filter(|(_, layer)| layer.matches_query(&self.composition_query))
            .collect();
        if search.lost_focus()
            && ui.input(|input| input.key_pressed(egui::Key::Enter))
            && let Some((_, layer)) = filtered.last()
        {
            self.execute(Command::SelectLayer { id: Some(layer.id) });
            self.composition_query.clear();
        }
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("OBJECT MAP · FRONT → BACK")
                    .size(9.0)
                    .strong()
                    .color(MUTED),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if !self.composition_query.is_empty() {
                    ui.label(
                        RichText::new(format!(
                            "{} match{}",
                            filtered.len(),
                            if filtered.len() == 1 { "" } else { "es" }
                        ))
                        .size(9.0)
                        .color(ACCENT),
                    );
                }
            });
        });
        egui::ScrollArea::vertical()
            .id_salt("composition-object-map")
            .max_height(260.0)
            .show(ui, |ui| {
                for (index, layer) in filtered.into_iter().rev() {
                    self.layer_row(ui, layer, index, layers.len());
                    ui.add_space(2.0);
                }
                if layers.is_empty() {
                    ui.add_space(18.0);
                    ui.vertical_centered(|ui| {
                        ui.label(RichText::new("The canvas has no objects yet.").color(MUTED));
                        ui.label(
                            RichText::new("Use ⌘K to add text or shapes, or place an image.")
                                .size(10.0)
                                .color(MUTED),
                        );
                    });
                } else if !self.composition_query.is_empty()
                    && !layers
                        .iter()
                        .any(|layer| layer.matches_query(&self.composition_query))
                {
                    ui.add_space(14.0);
                    ui.label(RichText::new("No object matches this query.").color(MUTED));
                }
            });
        if ui.input(|input| input.pointer.any_released())
            && let (Some(id), Some(index)) = (self.layer_drag.take(), self.layer_drop_index.take())
            && self
                .workspace
                .document
                .layers
                .iter()
                .position(|layer| layer.id == id)
                != Some(index)
        {
            self.execute(Command::MoveLayer { id, index });
        }
    }

    fn focus_card(&mut self, ui: &mut egui::Ui, layer: &LayerRowData, index: usize, total: usize) {
        let (ahead, behind) = stack_context(index, total);
        egui::Frame::new()
            .fill(Color32::from_rgb(31, 56, 57))
            .stroke(Stroke::new(1.0, ACCENT))
            .corner_radius(8.0)
            .inner_margin(10)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("FOCUS").size(9.0).strong().color(ACCENT));
                    ui.label(
                        RichText::new(format!("#{}", layer.id))
                            .monospace()
                            .size(9.0)
                            .color(MUTED),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(layer.label().to_uppercase())
                                .size(9.0)
                                .strong()
                                .color(MUTED),
                        );
                    });
                });
                ui.label(RichText::new(&layer.name).size(16.0).strong());
                ui.label(
                    RichText::new(format!(
                        "{} ahead · {} behind · position {}/{}",
                        ahead,
                        behind,
                        total - index,
                        total
                    ))
                    .size(10.0)
                    .color(MUTED),
                );
                ui.add_space(5.0);
                ui.horizontal(|ui| {
                    if ui.small_button("Duplicate").clicked() {
                        self.execute(Command::DuplicateLayer { id: layer.id });
                    }
                    if ui.small_button("Rename").clicked() {
                        self.rename_layer = Some((layer.id, layer.name.clone()));
                    }
                    if ui
                        .small_button(RichText::new("Delete").color(DANGER))
                        .clicked()
                    {
                        self.delete_confirmation = Some(layer.id);
                    }
                });
            });
    }

    fn layer_row(&mut self, ui: &mut egui::Ui, layer: &LayerRowData, index: usize, total: usize) {
        let selected = self.workspace.document.selected == Some(layer.id);
        let thumbnail = self.layer_thumbnail(ui.ctx(), layer.id, layer.raster_path());
        let row_height = 42.0;
        let thumbnail_size = 28.0;
        let (rect, response) = ui.allocate_exact_size(
            Vec2::new(ui.available_width(), row_height),
            Sense::click_and_drag(),
        );
        let dropping = self.layer_drag.is_some() && self.layer_drop_index == Some(index);
        ui.painter().rect(
            rect,
            5.0,
            if selected {
                Color32::from_rgb(36, 58, 60)
            } else {
                SURFACE
            },
            Stroke::new(
                if dropping { 2.0 } else { 1.0 },
                if selected || dropping { ACCENT } else { BORDER },
            ),
            egui::StrokeKind::Inside,
        );
        if selected {
            ui.painter().rect_filled(
                Rect::from_min_size(rect.min, Vec2::new(3.0, rect.height())),
                1.0,
                ACCENT,
            );
        }
        let controls = ui
            .scope_builder(
                egui::UiBuilder::new()
                    .max_rect(rect.shrink(6.0))
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
                |ui| {
                    let mut child_clicked = false;
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!("{:02}", total - index))
                                .monospace()
                                .size(10.0)
                                .color(if selected { ACCENT } else { MUTED }),
                        );
                        let kind = layer.label();
                        if let Some(texture) = thumbnail {
                            ui.add(
                                egui::Image::new(&texture)
                                    .fit_to_exact_size(Vec2::splat(thumbnail_size)),
                            );
                        } else {
                            let (rect, _) =
                                ui.allocate_exact_size(Vec2::splat(thumbnail_size), Sense::hover());
                            let fill = layer.fill();
                            ui.painter().rect_filled(rect, 4.0, fill);
                            ui.painter().text(
                                rect.center(),
                                Align2::CENTER_CENTER,
                                match kind {
                                    "Image" => "IMG",
                                    "Text" => "TXT",
                                    _ => "SHP",
                                },
                                FontId::monospace(8.0),
                                contrast_text(fill),
                            );
                        }
                        ui.vertical(|ui| {
                            ui.label(RichText::new(&layer.name).size(12.0).strong());
                            ui.label(
                                RichText::new(format!("{} · #{}", kind, layer.id))
                                    .size(9.0)
                                    .color(MUTED),
                            );
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if icon_toggle(ui, layer.locked, ToggleIcon::Lock)
                                .on_hover_text(if layer.locked {
                                    "Unlock layer"
                                } else {
                                    "Lock layer"
                                })
                                .clicked()
                            {
                                child_clicked = true;
                                self.execute(Command::SetLocked {
                                    id: layer.id,
                                    locked: !layer.locked,
                                });
                            }
                            if icon_toggle(ui, layer.visible, ToggleIcon::Visibility)
                                .on_hover_text("Toggle visibility")
                                .clicked()
                            {
                                child_clicked = true;
                                self.execute(Command::SetVisibility {
                                    id: layer.id,
                                    visible: !layer.visible,
                                });
                            }
                        });
                    });
                    child_clicked
                },
            )
            .inner;
        if response.clicked() && !controls {
            self.execute(Command::SelectLayer { id: Some(layer.id) });
        }
        if response.double_clicked() && !controls {
            self.rename_layer = Some((layer.id, layer.name.clone()));
        }
        if response.drag_started() {
            self.layer_drag = Some(layer.id);
            self.layer_drop_index = Some(index);
            self.execute(Command::SelectLayer { id: Some(layer.id) });
        }
        if response.hovered() && self.layer_drag.is_some() {
            self.layer_drop_index = Some(index);
        }
    }

    pub(super) fn layer_thumbnail(
        &mut self,
        context: &egui::Context,
        id: u64,
        path: Option<&Path>,
    ) -> Option<TextureHandle> {
        if let Some(texture) = self.layer_thumbnails.get(&id) {
            return Some(texture.clone());
        }
        let path = path?;
        let image = image::open(path).ok()?.thumbnail(96, 96).to_rgba8();
        let size = [image.width() as usize, image.height() as usize];
        let color = egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw());
        let texture =
            context.load_texture(format!("prism-layer-{id}"), color, TextureOptions::LINEAR);
        self.layer_thumbnails.insert(id, texture.clone());
        Some(texture)
    }
}

fn stack_context(index: usize, total: usize) -> (usize, usize) {
    (total.saturating_sub(index + 1), index)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stack_context_explains_focus_without_tree_regions() {
        assert_eq!(stack_context(2, 5), (2, 2));
        assert_eq!(stack_context(4, 5), (0, 4));
    }

    #[test]
    fn composition_search_matches_name_kind_and_stable_id() {
        let layer = LayerRowData {
            id: 42,
            name: "Hero title".into(),
            visible: true,
            locked: false,
            kind: LayerRowKind::Text([255, 255, 255, 255]),
        };
        assert!(layer.matches_query("hero"));
        assert!(layer.matches_query("text"));
        assert!(layer.matches_query("#42"));
        assert!(!layer.matches_query("image"));
    }
}
