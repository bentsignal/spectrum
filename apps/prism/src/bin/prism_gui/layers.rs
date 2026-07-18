use super::*;

const FOCUS_CARD_CONTENT_HEIGHT: f32 = 100.0;

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ObjectMapNavigation {
    Next,
    Previous,
    Confirm,
    Cancel,
}

fn step_result_index(current: usize, result_count: usize, forward: bool) -> usize {
    if result_count == 0 {
        0
    } else if forward {
        (current + 1) % result_count
    } else {
        (current + result_count - 1) % result_count
    }
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
        ui.add_space(8.0);
        let search = ui
            .horizontal(|ui| {
                let search_width = (ui.available_width() - 50.0).max(120.0);
                let search = ui.add_sized(
                    [search_width, 26.0],
                    egui::TextEdit::singleline(&mut self.composition_query)
                        .hint_text("Jump to object…")
                        .vertical_align(egui::Align::Center),
                );
                let _ = command_shortcut(ui, "J");
                search
            })
            .inner;
        let focus_requested = self.composition_search_focus;
        let mut scroll_to_result = focus_requested;
        if focus_requested {
            search.request_focus();
            self.composition_search_focus = false;
        }
        if search.changed() {
            self.composition_result_index = 0;
            scroll_to_result = true;
        }
        let layers: Vec<_> = self
            .workspace
            .document
            .layers
            .iter()
            .map(LayerRowData::from)
            .collect();
        let filtered: Vec<_> = layers
            .iter()
            .enumerate()
            .filter(|(_, layer)| layer.matches_query(&self.composition_query))
            .rev()
            .collect();
        if focus_requested {
            self.composition_result_index = self
                .workspace
                .document
                .selected
                .and_then(|selected| filtered.iter().position(|(_, layer)| layer.id == selected))
                .unwrap_or(0);
        }
        self.composition_result_index = self
            .composition_result_index
            .min(filtered.len().saturating_sub(1));
        let search_active = search.has_focus() || search.lost_focus();
        let navigation = ui.input(|input| {
            if input.key_pressed(egui::Key::ArrowDown) {
                Some(ObjectMapNavigation::Next)
            } else if input.key_pressed(egui::Key::ArrowUp) {
                Some(ObjectMapNavigation::Previous)
            } else if input.key_pressed(egui::Key::Enter) {
                Some(ObjectMapNavigation::Confirm)
            } else if input.key_pressed(egui::Key::Escape) {
                Some(ObjectMapNavigation::Cancel)
            } else {
                None
            }
        });
        if search_active {
            match navigation {
                Some(ObjectMapNavigation::Next) => {
                    self.composition_result_index =
                        step_result_index(self.composition_result_index, filtered.len(), true);
                    scroll_to_result = true;
                }
                Some(ObjectMapNavigation::Previous) => {
                    self.composition_result_index =
                        step_result_index(self.composition_result_index, filtered.len(), false);
                    scroll_to_result = true;
                }
                Some(ObjectMapNavigation::Confirm) => {
                    if let Some((_, layer)) = filtered.get(self.composition_result_index) {
                        self.execute(Command::SelectLayer { id: Some(layer.id) });
                        self.composition_query.clear();
                        self.composition_result_index = 0;
                        search.surrender_focus();
                    }
                }
                Some(ObjectMapNavigation::Cancel) => {
                    self.composition_query.clear();
                    self.composition_result_index = 0;
                    search.surrender_focus();
                }
                None => {}
            }
        }
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("OBJECT MAP").size(9.0).strong().color(MUTED));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if search.has_focus() && !filtered.is_empty() {
                    ui.label(
                        RichText::new(format!(
                            "{} / {}",
                            self.composition_result_index + 1,
                            filtered.len()
                        ))
                        .size(9.0)
                        .color(ACCENT_WARM),
                    );
                } else if !self.composition_query.is_empty() {
                    ui.label(
                        RichText::new(format!("{} matches", filtered.len()))
                            .size(9.0)
                            .color(ACCENT),
                    );
                }
            });
        });
        let keyboard_target = search.has_focus().then_some(self.composition_result_index);
        egui::ScrollArea::vertical()
            .id_salt("composition-object-map")
            .max_height(260.0)
            .show(ui, |ui| {
                for (result_index, (index, layer)) in filtered.into_iter().enumerate() {
                    self.layer_row(
                        ui,
                        layer,
                        index,
                        layers.len(),
                        keyboard_target == Some(result_index),
                        scroll_to_result && keyboard_target == Some(result_index),
                    );
                    ui.add_space(2.0);
                }
                if layers.is_empty() {
                    ui.add_space(18.0);
                    ui.vertical_centered(|ui| {
                        ui.label(RichText::new("The canvas has no objects yet.").color(MUTED));
                        ui.label(
                            RichText::new("Open Tools & Actions to add text, shapes, or an image.")
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
        let selected_index = self.workspace.document.selected.and_then(|selected| {
            self.workspace
                .document
                .layers
                .iter()
                .position(|layer| layer.id == selected)
        });
        ui.add_space(10.0);
        if let (Some(index), Some(layer)) = (
            selected_index,
            self.workspace
                .document
                .selected
                .and_then(|id| layers.iter().find(|layer| layer.id == id)),
        ) {
            self.focus_card(ui, layer, index, layers.len());
        } else {
            self.empty_focus_card(ui);
        }
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
        egui::Frame::new()
            .fill(FOCUS_SURFACE)
            .stroke(Stroke::new(1.0, ACCENT))
            .corner_radius(8.0)
            .inner_margin(10)
            .show(ui, |ui| {
                ui.set_height(FOCUS_CARD_CONTENT_HEIGHT);
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
                    RichText::new(format!("Stack position {} / {}", total - index, total))
                        .size(10.0)
                        .color(MUTED),
                );
                ui.add_space(5.0);
                ui.horizontal(|ui| {
                    if matches!(layer.kind, LayerRowKind::Text(_)) {
                        if ui.small_button("Edit text").clicked() {
                            self.open_text_editor(layer.id);
                        }
                        let _ = alternate_shortcut(ui, "E");
                    }
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

    fn empty_focus_card(&self, ui: &mut egui::Ui) {
        egui::Frame::new()
            .fill(SURFACE)
            .stroke(Stroke::new(1.0, BORDER))
            .corner_radius(8.0)
            .inner_margin(10)
            .show(ui, |ui| {
                ui.set_height(FOCUS_CARD_CONTENT_HEIGHT);
                ui.label(RichText::new("FOCUS").size(9.0).strong().color(MUTED));
                ui.add_space(16.0);
                ui.vertical_centered(|ui| {
                    ui.label(
                        RichText::new("Nothing in focus")
                            .size(16.0)
                            .strong()
                            .color(TEXT),
                    );
                    ui.label(
                        RichText::new("Choose an object from the map or canvas.")
                            .size(10.0)
                            .color(MUTED),
                    );
                });
            });
    }

    fn layer_row(
        &mut self,
        ui: &mut egui::Ui,
        layer: &LayerRowData,
        index: usize,
        total: usize,
        keyboard_target: bool,
        scroll_to_keyboard_target: bool,
    ) {
        let selected = self.workspace.document.selected == Some(layer.id);
        let thumbnail = self.layer_thumbnail(ui.ctx(), layer.id, layer.raster_path());
        let row_height = 56.0;
        let thumbnail_size = 32.0;
        let (rect, response) = ui.allocate_exact_size(
            Vec2::new(ui.available_width(), row_height),
            Sense::click_and_drag(),
        );
        let dropping = self.layer_drag.is_some() && self.layer_drop_index == Some(index);
        ui.painter().rect(
            rect,
            5.0,
            if selected {
                SELECTED_SURFACE
            } else if keyboard_target {
                RAISED
            } else {
                SURFACE
            },
            Stroke::new(
                if dropping { 2.0 } else { 1.0 },
                if selected || dropping {
                    ACCENT
                } else if keyboard_target {
                    ACCENT_WARM
                } else {
                    BORDER
                },
            ),
            egui::StrokeKind::Inside,
        );
        let inner = rect.shrink2(Vec2::new(8.0, 6.0));
        let number_rect = Rect::from_min_size(inner.min, Vec2::new(24.0, inner.height()));
        let thumbnail_rect = Rect::from_center_size(
            Pos2::new(number_rect.right() + 20.0, inner.center().y),
            Vec2::splat(thumbnail_size),
        );
        let controls_rect = Rect::from_min_max(
            Pos2::new(inner.right() - 60.0, inner.top()),
            inner.right_bottom(),
        );
        let text_rect = Rect::from_min_max(
            Pos2::new(thumbnail_rect.right() + 10.0, inner.center().y - 18.0),
            Pos2::new(controls_rect.left() - 8.0, inner.center().y + 18.0),
        );
        ui.painter().text(
            number_rect.center(),
            Align2::CENTER_CENTER,
            format!("{:02}", total - index),
            FontId::monospace(10.0),
            if selected { ACCENT } else { MUTED },
        );
        let kind = layer.label();
        if let Some(texture) = thumbnail {
            ui.painter().image(
                texture.id(),
                thumbnail_rect,
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                Color32::WHITE,
            );
        } else {
            let fill = layer.fill();
            ui.painter().rect_filled(thumbnail_rect, 4.0, fill);
            ui.painter().text(
                thumbnail_rect.center(),
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
        let text_painter = ui.painter().with_clip_rect(text_rect);
        text_painter.text(
            Pos2::new(text_rect.left(), text_rect.center().y - 8.0),
            Align2::LEFT_CENTER,
            &layer.name,
            FontId::proportional(12.0),
            TEXT,
        );
        text_painter.text(
            Pos2::new(text_rect.left(), text_rect.center().y + 8.0),
            Align2::LEFT_CENTER,
            format!("{} · #{}", kind, layer.id),
            FontId::proportional(9.0),
            MUTED,
        );
        let controls = ui
            .scope_builder(
                egui::UiBuilder::new()
                    .max_rect(controls_rect)
                    .layout(egui::Layout::right_to_left(egui::Align::Center)),
                |ui| {
                    let mut child_clicked = false;
                    ui.spacing_mut().item_spacing.x = 2.0;
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
        if scroll_to_keyboard_target {
            response.scroll_to_me(Some(egui::Align::Center));
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

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn object_map_navigation_wraps_in_both_directions() {
        assert_eq!(step_result_index(0, 3, true), 1);
        assert_eq!(step_result_index(2, 3, true), 0);
        assert_eq!(step_result_index(0, 3, false), 2);
        assert_eq!(step_result_index(0, 0, true), 0);
    }
}
