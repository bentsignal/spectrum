use prism_core::{Command, DropShadow, GradientStop, Layer, LayerStyle, ShapeFill, ShapeGradient};

use super::*;

impl PrismApp {
    pub(super) fn layer_effects_controls(&mut self, ui: &mut egui::Ui, layer: &Layer) {
        ui.separator();
        ui.label(RichText::new("LAYER STYLE").size(10.0).color(MUTED));
        let mut shadow_enabled = layer.style.drop_shadow.is_some();
        if ui.checkbox(&mut shadow_enabled, "Drop shadow").changed() {
            self.execute(Command::SetLayerStyle {
                id: layer.id,
                style: LayerStyle {
                    drop_shadow: shadow_enabled.then(DropShadow::default),
                },
            });
        }
        if let Some(mut shadow) = layer.style.drop_shadow {
            let response = ui.add(
                egui::Slider::new(&mut shadow.offset_x, -128.0..=128.0)
                    .text("Offset X")
                    .suffix(" px"),
            );
            self.widget_command(
                &response,
                Command::SetLayerStyle {
                    id: layer.id,
                    style: LayerStyle {
                        drop_shadow: Some(shadow),
                    },
                },
            );
            let response = ui.add(
                egui::Slider::new(&mut shadow.offset_y, -128.0..=128.0)
                    .text("Offset Y")
                    .suffix(" px"),
            );
            self.widget_command(
                &response,
                Command::SetLayerStyle {
                    id: layer.id,
                    style: LayerStyle {
                        drop_shadow: Some(shadow),
                    },
                },
            );
            let response = ui.add(
                egui::Slider::new(&mut shadow.blur_radius, 0.0..=128.0)
                    .text("Blur")
                    .suffix(" px"),
            );
            self.widget_command(
                &response,
                Command::SetLayerStyle {
                    id: layer.id,
                    style: LayerStyle {
                        drop_shadow: Some(shadow),
                    },
                },
            );
            let mut color = Color32::from_rgba_unmultiplied(
                shadow.color[0],
                shadow.color[1],
                shadow.color[2],
                shadow.color[3],
            );
            let response = ui.color_edit_button_srgba(&mut color);
            shadow.color = color.to_array();
            self.widget_command(
                &response,
                Command::SetLayerStyle {
                    id: layer.id,
                    style: LayerStyle {
                        drop_shadow: Some(shadow),
                    },
                },
            );
        }

        if matches!(
            layer.kind,
            prism_core::LayerKind::Rectangle { .. } | prism_core::LayerKind::Ellipse { .. }
        ) {
            self.shape_gradient_controls(ui, layer);
        }
    }

    fn shape_gradient_controls(&mut self, ui: &mut egui::Ui, layer: &Layer) {
        let mut enabled = layer.shape_fill.is_some();
        if ui.checkbox(&mut enabled, "Linear gradient").changed() {
            self.execute(Command::SetShapeFill {
                id: layer.id,
                fill: enabled.then(|| ShapeFill::Gradient(ShapeGradient::default())),
            });
        }
        let Some(ShapeFill::Gradient(mut gradient)) = layer.shape_fill.clone() else {
            return;
        };
        let response = ui.add(
            egui::Slider::new(&mut gradient.angle, 0.0..=360.0)
                .text("Angle")
                .suffix("°"),
        );
        self.widget_command(
            &response,
            Command::SetShapeFill {
                id: layer.id,
                fill: Some(ShapeFill::Gradient(gradient.clone())),
            },
        );
        for (index, label) in ["Start", "End"].into_iter().enumerate() {
            let mut color = Color32::from_rgba_unmultiplied(
                gradient.stops[index].color[0],
                gradient.stops[index].color[1],
                gradient.stops[index].color[2],
                gradient.stops[index].color[3],
            );
            ui.horizontal(|ui| {
                ui.label(label);
                let response = ui.color_edit_button_srgba(&mut color);
                gradient.stops[index] = GradientStop::new(index as f32, color.to_array());
                self.widget_command(
                    &response,
                    Command::SetShapeFill {
                        id: layer.id,
                        fill: Some(ShapeFill::Gradient(gradient.clone())),
                    },
                );
            });
        }
    }
}
