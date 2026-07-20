use super::*;

impl PrismApp {
    fn rasterize_shape(&mut self, id: u64) {
        let result = (|| {
            let layer = self.workspace.document.layer(id)?;
            let scale = prism_core::recommended_rasterization_scale(layer)?;
            let asset = prism_core::rasterize_shape_asset(&self.workspace.document, id, scale)?;
            Ok::<_, anyhow::Error>(Command::RasterizeShape {
                id,
                path: asset.path,
                scale: asset.scale,
            })
        })();
        match result {
            Ok(command) => {
                self.execute(command);
            }
            Err(error) => {
                self.status = format!("Rasterization failed: {error:#}");
                self.status_error = true;
            }
        }
    }

    pub(super) fn inspector(&mut self, ui: &mut egui::Ui) {
        let selected = self.selected_layer().cloned();
        ui.horizontal(|ui| {
            ui.label(RichText::new("PROPERTIES").size(11.0).strong().color(TEXT));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let label = selected
                    .as_ref()
                    .map_or("CANVAS", |layer| layer_kind_label(&layer.kind));
                ui.label(RichText::new(label).size(9.0).strong().color(ACCENT));
            });
        });
        ui.label(
            RichText::new(
                selected
                    .as_ref()
                    .map_or("Document dimensions and background", |layer| {
                        layer.name.as_str()
                    }),
            )
            .size(10.0)
            .color(MUTED),
        );
        ui.add_space(8.0);

        let Some(layer) = selected else {
            self.canvas_inspector(ui);
            return;
        };
        self.layer_header(ui, &layer);
        ui.add_space(8.0);
        egui::ScrollArea::vertical()
            .id_salt(("properties-inspector", layer.id))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                self.transform_inspector(ui, &layer);
                self.content_inspector(ui, &layer);
                self.appearance_inspector(ui, &layer);
                self.mask_inspector(ui, &layer);
                self.adjustments_inspector(ui, &layer);
            });
    }

    fn canvas_inspector(&mut self, ui: &mut egui::Ui) {
        egui::Frame::new()
            .fill(SURFACE)
            .stroke(Stroke::new(1.0, BORDER))
            .corner_radius(8.0)
            .inner_margin(12)
            .show(ui, |ui| {
                ui.label(
                    RichText::new(&self.workspace.document.name)
                        .size(15.0)
                        .strong()
                        .color(TEXT),
                );
                ui.label(
                    RichText::new(format!(
                        "{} × {} px · {} objects",
                        self.workspace.document.width,
                        self.workspace.document.height,
                        self.workspace.document.layers.len()
                    ))
                    .size(10.0)
                    .color(MUTED),
                );
            });
        ui.add_space(8.0);
        egui::CollapsingHeader::new("Canvas")
            .default_open(true)
            .show(ui, |ui| {
                let mut width = self.workspace.document.width;
                let mut height = self.workspace.document.height;
                let mut background = color32(self.workspace.document.background);
                egui::Grid::new("canvas-size-grid")
                    .num_columns(2)
                    .spacing(Vec2::new(10.0, 7.0))
                    .show(ui, |ui| {
                        property_label(ui, "Width");
                        let response = ui.add_sized(
                            [ui.available_width(), 24.0],
                            egui::DragValue::new(&mut width)
                                .range(1..=prism_core::MAX_CANVAS_DIMENSION)
                                .suffix(" px"),
                        );
                        self.widget_command(
                            &response,
                            Command::SetCanvas {
                                width,
                                height,
                                background: rgba(background),
                            },
                        );
                        ui.end_row();
                        property_label(ui, "Height");
                        let response = ui.add_sized(
                            [ui.available_width(), 24.0],
                            egui::DragValue::new(&mut height)
                                .range(1..=prism_core::MAX_CANVAS_DIMENSION)
                                .suffix(" px"),
                        );
                        self.widget_command(
                            &response,
                            Command::SetCanvas {
                                width,
                                height,
                                background: rgba(background),
                            },
                        );
                        ui.end_row();
                        property_label(ui, "Background");
                        let response = ui.color_edit_button_srgba(&mut background);
                        self.widget_command(
                            &response,
                            Command::SetCanvas {
                                width,
                                height,
                                background: rgba(background),
                            },
                        );
                        ui.end_row();
                    });
                ui.add_space(6.0);
                if ui.button("Crop canvas  C").clicked() {
                    self.choose_tool(Tool::Crop);
                }
            });
    }

    fn layer_header(&mut self, ui: &mut egui::Ui, layer: &Layer) {
        egui::Frame::new()
            .fill(SURFACE)
            .stroke(Stroke::new(1.0, BORDER))
            .corner_radius(8.0)
            .inner_margin(10)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let mut visible = layer.visible;
                    if ui.checkbox(&mut visible, "Visible").changed() {
                        self.execute(Command::SetVisibility {
                            id: layer.id,
                            visible,
                        });
                    }
                    let mut locked = layer.locked;
                    if ui.checkbox(&mut locked, "Locked").changed() {
                        self.execute(Command::SetLocked {
                            id: layer.id,
                            locked,
                        });
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(format!("#{}", layer.id))
                                .monospace()
                                .size(9.0)
                                .color(MUTED),
                        );
                    });
                });
                ui.add_space(5.0);
                ui.horizontal(|ui| {
                    if matches!(layer.kind, LayerKind::Text { .. }) {
                        if ui.small_button("Edit text").clicked() {
                            self.open_text_editor(layer.id);
                        }
                        let _ = alternate_shortcut(ui, "E");
                    }
                    if ui.small_button("Duplicate").clicked() {
                        self.execute(Command::DuplicateLayer { id: layer.id });
                    }
                    let _ = alternate_shortcut(ui, "D");
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

    fn transform_inspector(&mut self, ui: &mut egui::Ui, layer: &Layer) {
        egui::CollapsingHeader::new("Transform")
            .default_open(true)
            .show(ui, |ui| {
                let mut transform = layer.transform;
                let source = self.layer_source_size(layer);
                egui::Grid::new(("transform-grid", layer.id))
                    .num_columns(4)
                    .spacing(Vec2::new(6.0, 7.0))
                    .show(ui, |ui| {
                        property_label(ui, "X");
                        let response = ui.add(
                            egui::DragValue::new(&mut transform.x)
                                .speed(1.0)
                                .suffix(" px"),
                        );
                        self.widget_command(
                            &response,
                            Command::SetTransform {
                                id: layer.id,
                                transform,
                            },
                        );
                        property_label(ui, "Y");
                        let response = ui.add(
                            egui::DragValue::new(&mut transform.y)
                                .speed(1.0)
                                .suffix(" px"),
                        );
                        self.widget_command(
                            &response,
                            Command::SetTransform {
                                id: layer.id,
                                transform,
                            },
                        );
                        ui.end_row();

                        if let Some(source) = source {
                            let mut width = source.x * transform.scale_x;
                            let mut height = source.y * transform.scale_y;
                            property_label(ui, "W");
                            let response = ui.add(
                                egui::DragValue::new(&mut width)
                                    .speed(1.0)
                                    .range(1.0..=100_000.0)
                                    .suffix(" px"),
                            );
                            transform.scale_x = width / source.x.max(1.0);
                            self.widget_command(
                                &response,
                                Command::SetTransform {
                                    id: layer.id,
                                    transform,
                                },
                            );
                            property_label(ui, "H");
                            let response = ui.add(
                                egui::DragValue::new(&mut height)
                                    .speed(1.0)
                                    .range(1.0..=100_000.0)
                                    .suffix(" px"),
                            );
                            transform.scale_y = height / source.y.max(1.0);
                            self.widget_command(
                                &response,
                                Command::SetTransform {
                                    id: layer.id,
                                    transform,
                                },
                            );
                            ui.end_row();
                        }

                        property_label(ui, "Angle");
                        let response = ui.add(
                            egui::DragValue::new(&mut transform.rotation)
                                .speed(0.25)
                                .suffix("°"),
                        );
                        self.widget_command(
                            &response,
                            Command::SetTransform {
                                id: layer.id,
                                transform,
                            },
                        );
                        ui.end_row();
                    });
                ui.horizontal(|ui| {
                    if ui.small_button("Center on canvas").clicked() {
                        let source = source.unwrap_or(Vec2::splat(1.0));
                        self.execute(Command::SetTransform {
                            id: layer.id,
                            transform: Transform {
                                x: (self.workspace.document.width as f32
                                    - source.x * transform.scale_x)
                                    * 0.5,
                                y: (self.workspace.document.height as f32
                                    - source.y * transform.scale_y)
                                    * 0.5,
                                ..transform
                            },
                        });
                    }
                    if ui.small_button("Reset").clicked() {
                        self.execute(Command::SetTransform {
                            id: layer.id,
                            transform: Transform {
                                x: transform.x,
                                y: transform.y,
                                ..Default::default()
                            },
                        });
                    }
                });
            });
    }

    fn content_inspector(&mut self, ui: &mut egui::Ui, layer: &Layer) {
        egui::CollapsingHeader::new("Content")
            .default_open(true)
            .show(ui, |ui| {
                match &layer.kind {
                    LayerKind::Text {
                        text,
                        font_size,
                        color,
                    } => self.text_content(ui, layer.id, text, *font_size, *color),
                    LayerKind::Rectangle {
                        width,
                        height,
                        color,
                        corner_radius,
                    } => self.rectangle_content(
                        ui,
                        layer.id,
                        *width,
                        *height,
                        *color,
                        *corner_radius,
                    ),
                    LayerKind::Ellipse {
                        width,
                        height,
                        color,
                    } => self.ellipse_content(ui, layer.id, *width, *height, *color),
                    LayerKind::Raster { path, .. } => {
                        property_label(ui, "Linked image");
                        ui.label(
                            RichText::new(path.display().to_string())
                                .size(10.0)
                                .color(MUTED),
                        );
                    }
                }
                if matches!(
                    layer.kind,
                    LayerKind::Rectangle { .. } | LayerKind::Ellipse { .. }
                ) {
                    ui.separator();
                    if ui.button("Rasterize Shape").clicked() {
                        self.rasterize_shape(layer.id);
                    }
                    ui.label(
                        RichText::new(
                            "Freezes editable geometry into pixels at its current scale.",
                        )
                        .size(9.0)
                        .color(MUTED),
                    );
                }
            });
    }

    fn text_content(
        &mut self,
        ui: &mut egui::Ui,
        id: u64,
        current: &str,
        current_size: f32,
        current_color: [u8; 4],
    ) {
        let mut text = current.to_owned();
        let mut font_size = current_size;
        let mut color = color32(current_color);
        let response = ui.add(
            egui::TextEdit::multiline(&mut text)
                .desired_rows(3)
                .desired_width(f32::INFINITY),
        );
        self.widget_command_if(
            &response,
            (!text.trim().is_empty()).then(|| Command::UpdateText {
                id,
                text: text.clone(),
                font_size,
                color: rgba(color),
            }),
        );
        let response = ui.add(
            egui::Slider::new(&mut font_size, 4.0..=1_000.0)
                .text("Size")
                .suffix(" px"),
        );
        self.widget_command(
            &response,
            Command::UpdateText {
                id,
                text: text.clone(),
                font_size,
                color: rgba(color),
            },
        );
        let response = color_row(ui, "Color", &mut color);
        self.widget_command(
            &response,
            Command::UpdateText {
                id,
                text,
                font_size,
                color: rgba(color),
            },
        );
    }

    fn rectangle_content(
        &mut self,
        ui: &mut egui::Ui,
        id: u64,
        mut width: u32,
        mut height: u32,
        current_color: [u8; 4],
        mut radius: f32,
    ) {
        let mut color = color32(current_color);
        shape_size_grid(
            ui,
            id,
            &mut width,
            &mut height,
            |response, width, height| {
                self.widget_command(
                    response,
                    Command::UpdateRectangle {
                        id,
                        width,
                        height,
                        color: rgba(color),
                        corner_radius: radius,
                    },
                );
            },
        );
        let response = ui.add(
            egui::Slider::new(&mut radius, 0.0..=512.0)
                .text("Corner radius")
                .suffix(" px"),
        );
        self.widget_command(
            &response,
            Command::UpdateRectangle {
                id,
                width,
                height,
                color: rgba(color),
                corner_radius: radius,
            },
        );
        let response = color_row(ui, "Fill", &mut color);
        self.widget_command(
            &response,
            Command::UpdateRectangle {
                id,
                width,
                height,
                color: rgba(color),
                corner_radius: radius,
            },
        );
    }

    fn ellipse_content(
        &mut self,
        ui: &mut egui::Ui,
        id: u64,
        mut width: u32,
        mut height: u32,
        current_color: [u8; 4],
    ) {
        let mut color = color32(current_color);
        shape_size_grid(
            ui,
            id,
            &mut width,
            &mut height,
            |response, width, height| {
                self.widget_command(
                    response,
                    Command::UpdateEllipse {
                        id,
                        width,
                        height,
                        color: rgba(color),
                    },
                );
            },
        );
        let response = color_row(ui, "Fill", &mut color);
        self.widget_command(
            &response,
            Command::UpdateEllipse {
                id,
                width,
                height,
                color: rgba(color),
            },
        );
    }

    fn appearance_inspector(&mut self, ui: &mut egui::Ui, layer: &Layer) {
        egui::CollapsingHeader::new("Appearance")
            .default_open(true)
            .show(ui, |ui| {
                let mut opacity = layer.opacity * 100.0;
                let response = ui.add(
                    egui::Slider::new(&mut opacity, 0.0..=100.0)
                        .text("Opacity")
                        .suffix("%"),
                );
                self.widget_command(
                    &response,
                    Command::SetOpacity {
                        id: layer.id,
                        opacity: opacity / 100.0,
                    },
                );
                let mut blend = layer.blend_mode;
                egui::ComboBox::from_label("Blend mode")
                    .selected_text(blend.label())
                    .show_ui(ui, |ui| {
                        for mode in BlendMode::ALL {
                            ui.selectable_value(&mut blend, mode, mode.label())
                                .on_hover_text(mode.description());
                        }
                    });
                if blend != layer.blend_mode {
                    self.execute(Command::SetBlendMode {
                        id: layer.id,
                        blend_mode: blend,
                    });
                }
                let mut clipped = layer.clip_to_below;
                if ui.checkbox(&mut clipped, "Clip to object below").changed() {
                    self.execute(Command::SetClipping {
                        id: layer.id,
                        enabled: clipped,
                    });
                }
                if matches!(
                    layer.kind,
                    LayerKind::Rectangle { .. } | LayerKind::Ellipse { .. }
                ) {
                    ui.separator();
                    self.shape_stroke(ui, layer);
                }
            });
    }

    fn shape_stroke(&mut self, ui: &mut egui::Ui, layer: &Layer) {
        let mut stroke = layer.stroke;
        if ui.checkbox(&mut stroke.enabled, "Inside stroke").changed() {
            self.execute(Command::SetShapeStroke {
                id: layer.id,
                stroke,
            });
        }
        if stroke.enabled {
            let response = ui.add(
                egui::Slider::new(&mut stroke.width, 0.5..=128.0)
                    .text("Width")
                    .suffix(" px"),
            );
            self.widget_command(
                &response,
                Command::SetShapeStroke {
                    id: layer.id,
                    stroke,
                },
            );
            let mut color = color32(stroke.color);
            let response = color_row(ui, "Color", &mut color);
            stroke.color = rgba(color);
            self.widget_command(
                &response,
                Command::SetShapeStroke {
                    id: layer.id,
                    stroke,
                },
            );
        }
    }

    fn mask_inspector(&mut self, ui: &mut egui::Ui, layer: &Layer) {
        egui::CollapsingHeader::new("Mask")
            .default_open(layer.mask.enabled)
            .show(ui, |ui| {
                let mut mask = layer.mask;
                if ui.checkbox(&mut mask.enabled, "Enable mask").changed() {
                    self.execute(Command::SetMask { id: layer.id, mask });
                }
                if mask.enabled {
                    let response = ui.add(egui::Slider::new(&mut mask.x, 0.0..=1.0).text("X"));
                    self.widget_command(&response, Command::SetMask { id: layer.id, mask });
                    let response = ui.add(egui::Slider::new(&mut mask.y, 0.0..=1.0).text("Y"));
                    self.widget_command(&response, Command::SetMask { id: layer.id, mask });
                    let response =
                        ui.add(egui::Slider::new(&mut mask.width, 0.0..=1.0).text("Width"));
                    self.widget_command(&response, Command::SetMask { id: layer.id, mask });
                    let response =
                        ui.add(egui::Slider::new(&mut mask.height, 0.0..=1.0).text("Height"));
                    self.widget_command(&response, Command::SetMask { id: layer.id, mask });
                    if ui.checkbox(&mut mask.invert, "Invert mask").changed() {
                        self.execute(Command::SetMask { id: layer.id, mask });
                    }
                    if ui.button("Redraw on canvas  M").clicked() {
                        self.choose_tool(Tool::Mask);
                    }
                }
            });
    }

    fn adjustments_inspector(&mut self, ui: &mut egui::Ui, layer: &Layer) {
        egui::CollapsingHeader::new("Develop")
            .default_open(false)
            .show(ui, |ui| {
                let a = &layer.adjustments;
                section_label(ui, "LIGHT");
                self.adjustment_slider(ui, layer.id, "Exposure", a.exposure, -5.0..=5.0, |v| {
                    AdjustmentPatch {
                        exposure: Some(v),
                        ..Default::default()
                    }
                });
                self.adjustment_slider(ui, layer.id, "Contrast", a.contrast, -100.0..=100.0, |v| {
                    AdjustmentPatch {
                        contrast: Some(v),
                        ..Default::default()
                    }
                });
                self.adjustment_slider(
                    ui,
                    layer.id,
                    "Highlights",
                    a.highlights,
                    -100.0..=100.0,
                    |v| AdjustmentPatch {
                        highlights: Some(v),
                        ..Default::default()
                    },
                );
                self.adjustment_slider(ui, layer.id, "Shadows", a.shadows, -100.0..=100.0, |v| {
                    AdjustmentPatch {
                        shadows: Some(v),
                        ..Default::default()
                    }
                });
                self.adjustment_slider(ui, layer.id, "Whites", a.whites, -100.0..=100.0, |v| {
                    AdjustmentPatch {
                        whites: Some(v),
                        ..Default::default()
                    }
                });
                self.adjustment_slider(ui, layer.id, "Blacks", a.blacks, -100.0..=100.0, |v| {
                    AdjustmentPatch {
                        blacks: Some(v),
                        ..Default::default()
                    }
                });
                section_label(ui, "COLOR");
                self.adjustment_slider(
                    ui,
                    layer.id,
                    "Temperature",
                    a.temperature,
                    -100.0..=100.0,
                    |v| AdjustmentPatch {
                        temperature: Some(v),
                        ..Default::default()
                    },
                );
                self.adjustment_slider(ui, layer.id, "Tint", a.tint, -100.0..=100.0, |v| {
                    AdjustmentPatch {
                        tint: Some(v),
                        ..Default::default()
                    }
                });
                self.adjustment_slider(ui, layer.id, "Vibrance", a.vibrance, -100.0..=100.0, |v| {
                    AdjustmentPatch {
                        vibrance: Some(v),
                        ..Default::default()
                    }
                });
                self.adjustment_slider(
                    ui,
                    layer.id,
                    "Saturation",
                    a.saturation,
                    -100.0..=100.0,
                    |v| AdjustmentPatch {
                        saturation: Some(v),
                        ..Default::default()
                    },
                );
                section_label(ui, "PRESENCE & DETAIL");
                self.adjustment_slider(ui, layer.id, "Texture", a.texture, -100.0..=100.0, |v| {
                    AdjustmentPatch {
                        texture: Some(v),
                        ..Default::default()
                    }
                });
                self.adjustment_slider(ui, layer.id, "Clarity", a.clarity, -100.0..=100.0, |v| {
                    AdjustmentPatch {
                        clarity: Some(v),
                        ..Default::default()
                    }
                });
                self.adjustment_slider(ui, layer.id, "Dehaze", a.dehaze, -100.0..=100.0, |v| {
                    AdjustmentPatch {
                        dehaze: Some(v),
                        ..Default::default()
                    }
                });
                self.adjustment_slider(ui, layer.id, "Vignette", a.vignette, -100.0..=100.0, |v| {
                    AdjustmentPatch {
                        vignette: Some(v),
                        ..Default::default()
                    }
                });
                self.adjustment_slider(
                    ui,
                    layer.id,
                    "Sharpening",
                    a.sharpening,
                    0.0..=100.0,
                    |v| AdjustmentPatch {
                        sharpening: Some(v),
                        ..Default::default()
                    },
                );
                self.adjustment_slider(
                    ui,
                    layer.id,
                    "Noise reduction",
                    a.noise_reduction,
                    0.0..=100.0,
                    |v| AdjustmentPatch {
                        noise_reduction: Some(v),
                        ..Default::default()
                    },
                );
                ui.add_space(6.0);
                if ui.button("Reset development").clicked() {
                    self.execute(Command::ResetLayerAdjustments { id: layer.id });
                }
            });
    }

    fn adjustment_slider(
        &mut self,
        ui: &mut egui::Ui,
        id: u64,
        label: &str,
        current: f32,
        range: std::ops::RangeInclusive<f32>,
        patch: impl FnOnce(f32) -> AdjustmentPatch,
    ) {
        let mut value = current;
        let response = ui.add(egui::Slider::new(&mut value, range).text(label));
        self.widget_command(
            &response,
            Command::AdjustLayer {
                id,
                patch: patch(value),
            },
        );
    }
}

fn layer_kind_label(kind: &LayerKind) -> &'static str {
    match kind {
        LayerKind::Raster { .. } => "IMAGE",
        LayerKind::Text { .. } => "TEXT",
        LayerKind::Rectangle { .. } => "RECTANGLE",
        LayerKind::Ellipse { .. } => "ELLIPSE",
    }
}

fn property_label(ui: &mut egui::Ui, label: &str) {
    ui.label(RichText::new(label).size(10.0).color(MUTED));
}

fn section_label(ui: &mut egui::Ui, label: &str) {
    ui.add_space(8.0);
    ui.label(RichText::new(label).size(9.0).strong().color(MUTED));
}

fn shape_size_grid(
    ui: &mut egui::Ui,
    id: u64,
    width: &mut u32,
    height: &mut u32,
    mut changed: impl FnMut(&egui::Response, u32, u32),
) {
    egui::Grid::new(("shape-size", id))
        .num_columns(4)
        .spacing(Vec2::new(6.0, 7.0))
        .show(ui, |ui| {
            property_label(ui, "W");
            let response = ui.add(
                egui::DragValue::new(width)
                    .range(1..=prism_core::MAX_CANVAS_DIMENSION)
                    .suffix(" px"),
            );
            changed(&response, *width, *height);
            property_label(ui, "H");
            let response = ui.add(
                egui::DragValue::new(height)
                    .range(1..=prism_core::MAX_CANVAS_DIMENSION)
                    .suffix(" px"),
            );
            changed(&response, *width, *height);
            ui.end_row();
        });
}

fn color_row(ui: &mut egui::Ui, label: &str, color: &mut Color32) -> egui::Response {
    ui.horizontal(|ui| {
        property_label(ui, label);
        ui.color_edit_button_srgba(color)
    })
    .inner
}
