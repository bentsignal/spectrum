use super::*;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(super) enum InspectorLens {
    #[default]
    Object,
    Look,
    Develop,
}

impl InspectorLens {
    const ALL: [Self; 3] = [Self::Object, Self::Look, Self::Develop];

    fn label(self) -> &'static str {
        match self {
            Self::Object => "Object",
            Self::Look => "Look",
            Self::Develop => "Develop",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::Object => "Geometry and intrinsic content",
            Self::Look => "Compositing, opacity, and masks",
            Self::Develop => "Nondestructive image treatment",
        }
    }
}

impl PrismApp {
    pub(super) fn inspector(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("INSPECTOR").size(11.0).strong());
        ui.label(
            RichText::new("Controls for the current lens")
                .size(9.0)
                .color(MUTED),
        );
        ui.add_space(8.0);
        let Some(layer) = self.selected_layer().cloned() else {
            ui.add_space(18.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("No object in focus").strong().color(MUTED));
                ui.label(
                    RichText::new("Choose one on canvas or use Jump to Object.")
                        .size(11.0)
                        .color(MUTED),
                );
            });
            return;
        };
        ui.horizontal(|ui| {
            for lens in InspectorLens::ALL {
                if ui
                    .selectable_label(
                        self.inspector_lens == lens,
                        RichText::new(lens.label()).strong(),
                    )
                    .clicked()
                {
                    self.inspector_lens = lens;
                }
            }
        });
        ui.label(
            RichText::new(self.inspector_lens.description())
                .size(10.0)
                .color(MUTED),
        );
        ui.add_space(6.0);
        egui::ScrollArea::vertical()
            .id_salt(("capability-inspector", self.inspector_lens))
            .show(ui, |ui| match self.inspector_lens {
                InspectorLens::Object => {
                    self.transform_inspector(ui, &layer);
                    ui.add_space(10.0);
                    self.content_inspector(ui, &layer);
                }
                InspectorLens::Look => self.appearance_inspector(ui, &layer),
                InspectorLens::Develop => self.adjustments_inspector(ui, &layer),
            });
    }

    pub(super) fn transform_inspector(&mut self, ui: &mut egui::Ui, layer: &Layer) {
        egui::CollapsingHeader::new("Geometry")
            .default_open(true)
            .show(ui, |ui| {
                let mut transform = layer.transform;
                ui.horizontal(|ui| {
                    ui.label("X");
                    let response = ui.add(egui::DragValue::new(&mut transform.x).speed(1.0));
                    self.widget_command(
                        &response,
                        Command::SetTransform {
                            id: layer.id,
                            transform,
                        },
                    );
                    ui.label("Y");
                    let response = ui.add(egui::DragValue::new(&mut transform.y).speed(1.0));
                    self.widget_command(
                        &response,
                        Command::SetTransform {
                            id: layer.id,
                            transform,
                        },
                    );
                });
                ui.horizontal(|ui| {
                    ui.label("Scale X");
                    let response = ui.add(
                        egui::DragValue::new(&mut transform.scale_x)
                            .speed(0.01)
                            .range(0.01..=100.0),
                    );
                    self.widget_command(
                        &response,
                        Command::SetTransform {
                            id: layer.id,
                            transform,
                        },
                    );
                    ui.label("Scale Y");
                    let response = ui.add(
                        egui::DragValue::new(&mut transform.scale_y)
                            .speed(0.01)
                            .range(0.01..=100.0),
                    );
                    self.widget_command(
                        &response,
                        Command::SetTransform {
                            id: layer.id,
                            transform,
                        },
                    );
                });
                ui.horizontal(|ui| {
                    ui.label("Angle");
                    let response = ui.add(
                        egui::DragValue::new(&mut transform.rotation)
                            .speed(0.25)
                            .suffix(" deg"),
                    );
                    self.widget_command(
                        &response,
                        Command::SetTransform {
                            id: layer.id,
                            transform,
                        },
                    );
                });
            });
    }

    pub(super) fn content_inspector(&mut self, ui: &mut egui::Ui, layer: &Layer) {
        match &layer.kind {
            LayerKind::Text {
                text,
                font_size,
                color,
            } => {
                egui::CollapsingHeader::new("Content")
                    .default_open(true)
                    .show(ui, |ui| {
                        let mut text = text.clone();
                        let mut font_size = *font_size;
                        let mut color = color32(*color);
                        let mut changed = false;
                        ui.label(RichText::new("Text").size(11.0).color(MUTED));
                        changed |= ui
                            .add(
                                egui::TextEdit::multiline(&mut text)
                                    .desired_rows(3)
                                    .desired_width(f32::INFINITY),
                            )
                            .changed();
                        changed |= ui
                            .add(
                                egui::Slider::new(&mut font_size, 4.0..=1_000.0)
                                    .text("Font size")
                                    .suffix(" px"),
                            )
                            .changed();
                        ui.horizontal(|ui| {
                            ui.label("Color");
                            changed |= ui.color_edit_button_srgba(&mut color).changed();
                        });
                        if changed && !text.trim().is_empty() {
                            self.execute(Command::UpdateText {
                                id: layer.id,
                                text,
                                font_size,
                                color: rgba(color),
                            });
                        }
                    });
            }
            LayerKind::Rectangle {
                width,
                height,
                color,
                corner_radius,
            } => {
                egui::CollapsingHeader::new("Content")
                    .default_open(true)
                    .show(ui, |ui| {
                        let mut width = *width;
                        let mut height = *height;
                        let mut color = color32(*color);
                        let mut corner_radius = *corner_radius;
                        let mut changed = false;
                        ui.horizontal(|ui| {
                            ui.label("Width");
                            changed |= ui
                                .add(egui::DragValue::new(&mut width).range(1..=32_768))
                                .changed();
                            ui.label("Height");
                            changed |= ui
                                .add(egui::DragValue::new(&mut height).range(1..=32_768))
                                .changed();
                        });
                        changed |= ui
                            .add(
                                egui::Slider::new(&mut corner_radius, 0.0..=512.0)
                                    .text("Corner radius")
                                    .suffix(" px"),
                            )
                            .changed();
                        ui.horizontal(|ui| {
                            ui.label("Fill");
                            changed |= ui.color_edit_button_srgba(&mut color).changed();
                        });
                        if changed {
                            self.execute(Command::UpdateRectangle {
                                id: layer.id,
                                width,
                                height,
                                color: rgba(color),
                                corner_radius,
                            });
                        }
                    });
            }
            LayerKind::Raster { path, .. } => {
                egui::CollapsingHeader::new("Content")
                    .default_open(false)
                    .show(ui, |ui| {
                        ui.label(RichText::new("Linked image").size(11.0).color(MUTED));
                        ui.label(
                            RichText::new(path.display().to_string())
                                .size(11.0)
                                .color(TEXT),
                        );
                    });
            }
        }
    }

    pub(super) fn appearance_inspector(&mut self, ui: &mut egui::Ui, layer: &Layer) {
        egui::CollapsingHeader::new("Compositing")
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
                egui::ComboBox::from_label("Blend")
                    .selected_text(blend.label())
                    .show_ui(ui, |ui| {
                        for mode in BlendMode::ALL {
                            ui.selectable_value(&mut blend, mode, mode.label())
                                .on_hover_text(mode.description());
                        }
                    });
                ui.label(RichText::new(blend.description()).size(10.0).color(MUTED));
                if blend != layer.blend_mode {
                    self.execute(Command::SetBlendMode {
                        id: layer.id,
                        blend_mode: blend,
                    });
                }
                let mut clipped = layer.clip_to_below;
                if ui.checkbox(&mut clipped, "Clip to layer below").changed() {
                    self.execute(Command::SetClipping {
                        id: layer.id,
                        enabled: clipped,
                    });
                }
                let mut mask = layer.mask;
                if ui
                    .checkbox(&mut mask.enabled, "Enable rectangular mask")
                    .changed()
                {
                    self.execute(Command::SetMask { id: layer.id, mask });
                }
                if mask.enabled {
                    let mut changed = false;
                    changed |= ui
                        .add(egui::Slider::new(&mut mask.x, 0.0..=0.99).text("Mask X"))
                        .changed();
                    changed |= ui
                        .add(egui::Slider::new(&mut mask.y, 0.0..=0.99).text("Mask Y"))
                        .changed();
                    changed |= ui
                        .add(egui::Slider::new(&mut mask.width, 0.01..=1.0).text("Mask width"))
                        .changed();
                    changed |= ui
                        .add(egui::Slider::new(&mut mask.height, 0.01..=1.0).text("Mask height"))
                        .changed();
                    changed |= ui.checkbox(&mut mask.invert, "Invert mask").changed();
                    if changed {
                        self.execute(Command::SetMask { id: layer.id, mask });
                    }
                }
            });
    }

    pub(super) fn adjustments_inspector(&mut self, ui: &mut egui::Ui, layer: &Layer) {
        egui::CollapsingHeader::new("Adjustments")
            .default_open(true)
            .show(ui, |ui| {
                let a = &layer.adjustments;
                self.adjustment_slider(ui, layer.id, "Exposure", a.exposure, -5.0..=5.0, |value| {
                    AdjustmentPatch {
                        exposure: Some(value),
                        ..Default::default()
                    }
                });
                self.adjustment_slider(
                    ui,
                    layer.id,
                    "Contrast",
                    a.contrast,
                    -100.0..=100.0,
                    |value| AdjustmentPatch {
                        contrast: Some(value),
                        ..Default::default()
                    },
                );
                self.adjustment_slider(
                    ui,
                    layer.id,
                    "Highlights",
                    a.highlights,
                    -100.0..=100.0,
                    |value| AdjustmentPatch {
                        highlights: Some(value),
                        ..Default::default()
                    },
                );
                self.adjustment_slider(
                    ui,
                    layer.id,
                    "Shadows",
                    a.shadows,
                    -100.0..=100.0,
                    |value| AdjustmentPatch {
                        shadows: Some(value),
                        ..Default::default()
                    },
                );
                self.adjustment_slider(
                    ui,
                    layer.id,
                    "Temperature",
                    a.temperature,
                    -100.0..=100.0,
                    |value| AdjustmentPatch {
                        temperature: Some(value),
                        ..Default::default()
                    },
                );
                self.adjustment_slider(
                    ui,
                    layer.id,
                    "Vibrance",
                    a.vibrance,
                    -100.0..=100.0,
                    |value| AdjustmentPatch {
                        vibrance: Some(value),
                        ..Default::default()
                    },
                );
                self.adjustment_slider(
                    ui,
                    layer.id,
                    "Saturation",
                    a.saturation,
                    -100.0..=100.0,
                    |value| AdjustmentPatch {
                        saturation: Some(value),
                        ..Default::default()
                    },
                );
                if ui.button("Reset layer development").clicked() {
                    self.execute(Command::ResetLayerAdjustments { id: layer.id });
                }
            });
    }

    pub(super) fn adjustment_slider(
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
