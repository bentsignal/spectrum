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
            LayerRowKind::Raster(_) => "IMG",
            LayerRowKind::Text(_) => "TXT",
            LayerRowKind::Shape(_) => "SHP",
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
}

impl PrismApp {
    pub(super) fn layers_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("LAYERS").size(10.0).strong().color(MUTED));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("+").on_hover_text("Place image").clicked() {
                    self.add_raster();
                }
            });
        });
        ui.add_space(6.0);
        let layers: Vec<_> = self
            .workspace
            .document
            .layers
            .iter()
            .map(LayerRowData::from)
            .collect();
        egui::ScrollArea::vertical()
            .id_salt("layer-stack")
            .max_height(270.0)
            .show(ui, |ui| {
                for (index, layer) in layers.iter().enumerate().rev() {
                    self.layer_row(ui, layer, index);
                    ui.add_space(3.0);
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
        ui.horizontal(|ui| {
            let selected = self.workspace.document.selected;
            if ui
                .add_enabled(selected.is_some(), egui::Button::new("Duplicate"))
                .clicked()
                && let Some(id) = selected
            {
                self.execute(Command::DuplicateLayer { id });
            }
            if ui
                .add_enabled(selected.is_some(), egui::Button::new("Delete"))
                .clicked()
            {
                self.delete_confirmation = selected;
            }
        });
    }

    fn layer_row(&mut self, ui: &mut egui::Ui, layer: &LayerRowData, index: usize) {
        let selected = self.workspace.document.selected == Some(layer.id);
        let thumbnail = self.layer_thumbnail(ui.ctx(), layer.id, layer.raster_path());
        let (rect, response) = ui.allocate_exact_size(
            Vec2::new(ui.available_width(), 46.0),
            Sense::click_and_drag(),
        );
        let dropping = self.layer_drag.is_some() && self.layer_drop_index == Some(index);
        ui.painter().rect(
            rect,
            6.0,
            if selected {
                Color32::from_rgb(38, 67, 67)
            } else {
                SURFACE
            },
            Stroke::new(
                if dropping { 2.0 } else { 1.0 },
                if selected || dropping { ACCENT } else { BORDER },
            ),
            egui::StrokeKind::Inside,
        );
        let controls = ui
            .scope_builder(
                egui::UiBuilder::new()
                    .max_rect(rect.shrink(6.0))
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
                |ui| {
                    let mut child_clicked = false;
                    ui.horizontal(|ui| {
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
                        let kind = layer.label();
                        if let Some(texture) = thumbnail {
                            ui.add(egui::Image::new(&texture).fit_to_exact_size(Vec2::splat(30.0)));
                        } else {
                            let (rect, _) =
                                ui.allocate_exact_size(Vec2::splat(30.0), Sense::hover());
                            let fill = layer.fill();
                            ui.painter().rect_filled(rect, 4.0, fill);
                            ui.painter().text(
                                rect.center(),
                                Align2::CENTER_CENTER,
                                kind,
                                FontId::monospace(8.0),
                                contrast_text(fill),
                            );
                        }
                        ui.label(RichText::new(format!("{kind}  {}", layer.name)).size(12.0));
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
