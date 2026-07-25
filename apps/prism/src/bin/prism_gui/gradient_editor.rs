use prism_core::{
    Command, GradientKind, GradientSpread, GradientStop, Layer, MAX_GRADIENT_STOPS, ShapeFill,
    ShapeGradient,
};

use super::*;

pub(super) fn gradient_editor(
    app: &mut PrismApp,
    ui: &mut egui::Ui,
    layer: &Layer,
    mut gradient: ShapeGradient,
) {
    ui.horizontal(|ui| {
        ui.label("Geometry");
        let before = gradient.kind;
        egui::ComboBox::from_id_salt(("gradient-kind", layer.id))
            .selected_text(kind_label(gradient.kind))
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut gradient.kind, GradientKind::Linear, "Linear");
                ui.selectable_value(&mut gradient.kind, GradientKind::Radial, "Radial");
                ui.selectable_value(&mut gradient.kind, GradientKind::Angle, "Angle");
            });
        if gradient.kind != before {
            execute_gradient(app, layer.id, &gradient);
        }
    });
    if matches!(gradient.kind, GradientKind::Linear | GradientKind::Angle) {
        let response = ui.add(
            egui::Slider::new(&mut gradient.angle, 0.0..=360.0)
                .text("Angle")
                .suffix("°"),
        );
        widget_gradient(app, &response, layer.id, &gradient);
        let response = ui.add(egui::Slider::new(&mut gradient.offset, -2.0..=2.0).text("Offset"));
        widget_gradient(app, &response, layer.id, &gradient);
        let response = ui.add(
            egui::Slider::new(&mut gradient.extent, 0.01..=4.0)
                .text("Extent")
                .logarithmic(true),
        );
        widget_gradient(app, &response, layer.id, &gradient);
    }
    if matches!(gradient.kind, GradientKind::Radial | GradientKind::Angle) {
        let response =
            ui.add(egui::Slider::new(&mut gradient.center[0], 0.0..=1.0).text("Center X"));
        widget_gradient(app, &response, layer.id, &gradient);
        let response =
            ui.add(egui::Slider::new(&mut gradient.center[1], 0.0..=1.0).text("Center Y"));
        widget_gradient(app, &response, layer.id, &gradient);
    }
    if gradient.kind == GradientKind::Radial {
        let response = ui.add(
            egui::Slider::new(&mut gradient.radius, 0.01..=2.0)
                .text("Radius")
                .logarithmic(true),
        );
        widget_gradient(app, &response, layer.id, &gradient);
    }
    ui.horizontal(|ui| {
        ui.label("Spread");
        let before = gradient.spread;
        egui::ComboBox::from_id_salt(("gradient-spread", layer.id))
            .selected_text(spread_label(gradient.spread))
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut gradient.spread, GradientSpread::Pad, "Pad");
                ui.selectable_value(&mut gradient.spread, GradientSpread::Repeat, "Repeat");
                ui.selectable_value(&mut gradient.spread, GradientSpread::Reflect, "Reflect");
            });
        if gradient.spread != before {
            execute_gradient(app, layer.id, &gradient);
        }
    });

    ui.label(
        RichText::new(format!(
            "Stops ({}/{MAX_GRADIENT_STOPS})",
            gradient.stops.len()
        ))
        .size(9.0)
        .color(MUTED),
    );
    ui.small("Premultiplied sRGB v1");
    let mut remove = None;
    for index in 0..gradient.stops.len() {
        ui.horizontal(|ui| {
            let lower = if index == 0 {
                0.0
            } else {
                gradient.stops[index - 1].position + 0.001
            };
            let upper = if index + 1 == gradient.stops.len() {
                1.0
            } else {
                gradient.stops[index + 1].position - 0.001
            };
            let response = ui.add(
                egui::DragValue::new(&mut gradient.stops[index].position)
                    .range(lower..=upper)
                    .speed(0.005)
                    .max_decimals(3),
            );
            widget_gradient(app, &response, layer.id, &gradient);
            let mut color = Color32::from_rgba_unmultiplied(
                gradient.stops[index].color[0],
                gradient.stops[index].color[1],
                gradient.stops[index].color[2],
                gradient.stops[index].color[3],
            );
            let response = ui.color_edit_button_srgba(&mut color);
            gradient.stops[index].color = color.to_array();
            widget_gradient(app, &response, layer.id, &gradient);
            if gradient.stops.len() > 2
                && ui
                    .small_button("−")
                    .on_hover_text("Remove this stop")
                    .clicked()
            {
                remove = Some(index);
            }
        });
    }
    if let Some(index) = remove {
        gradient.stops.remove(index);
        execute_gradient(app, layer.id, &gradient);
    }
    if gradient.stops.len() < MAX_GRADIENT_STOPS
        && ui
            .small_button("+ Add stop")
            .on_hover_text("Insert a stop in the widest interval")
            .clicked()
    {
        let (index, position, color) = widest_gap_stop(&gradient);
        gradient
            .stops
            .insert(index, GradientStop::new(position, color));
        execute_gradient(app, layer.id, &gradient);
    }
}

fn execute_gradient(app: &mut PrismApp, id: u64, gradient: &ShapeGradient) {
    app.execute(Command::SetShapeFill {
        id,
        fill: Some(ShapeFill::Gradient(gradient.clone())),
    });
}

fn widget_gradient(
    app: &mut PrismApp,
    response: &egui::Response,
    id: u64,
    gradient: &ShapeGradient,
) {
    app.widget_command(
        response,
        Command::SetShapeFill {
            id,
            fill: Some(ShapeFill::Gradient(gradient.clone())),
        },
    );
}

fn widest_gap_stop(gradient: &ShapeGradient) -> (usize, f32, [u8; 4]) {
    let (index, pair) = gradient
        .stops
        .windows(2)
        .enumerate()
        .max_by(|(_, left), (_, right)| {
            (left[1].position - left[0].position)
                .total_cmp(&(right[1].position - right[0].position))
        })
        .expect("validated gradients have at least two stops");
    let position = (pair[0].position + pair[1].position) * 0.5;
    let color = gradient.sampler().sample_position(position);
    (index + 1, position, color)
}

fn kind_label(kind: GradientKind) -> &'static str {
    match kind {
        GradientKind::Linear => "Linear",
        GradientKind::Radial => "Radial",
        GradientKind::Angle => "Angle",
    }
}

fn spread_label(spread: GradientSpread) -> &'static str {
    match spread {
        GradientSpread::Pad => "Pad",
        GradientSpread::Repeat => "Repeat",
        GradientSpread::Reflect => "Reflect",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widest_gap_uses_the_exact_premultiplied_render_interpolation() {
        let gradient = ShapeGradient {
            stops: vec![
                GradientStop::new(0.0, [255, 0, 0, 255]),
                GradientStop::new(1.0, [0, 0, 255, 0]),
            ],
            ..Default::default()
        };
        assert_eq!(widest_gap_stop(&gradient), (1, 0.5, [255, 0, 0, 128]));
    }

    #[test]
    fn inserted_stop_is_one_command_without_a_preview_interaction() {
        let mut workspace =
            prism_core::Workspace::new(prism_core::Document::new("Gradient", 32, 24), None);
        workspace
            .execute(Command::AddRectangle {
                name: None,
                width: 16,
                height: 12,
                color: [255; 4],
                corner_radius: 0.0,
                x: 0.0,
                y: 0.0,
            })
            .unwrap();
        let mut gradient = ShapeGradient::default();
        let (index, position, color) = widest_gap_stop(&gradient);
        gradient
            .stops
            .insert(index, GradientStop::new(position, color));
        workspace
            .execute(Command::SetShapeFill {
                id: 1,
                fill: Some(ShapeFill::Gradient(gradient.clone())),
            })
            .unwrap();
        assert!(!workspace.interaction_active());
        assert_eq!(
            workspace.document.layer(1).unwrap().shape_fill,
            Some(ShapeFill::Gradient(gradient))
        );
        workspace.execute(Command::Undo).unwrap();
        assert!(workspace.document.layer(1).unwrap().shape_fill.is_none());
        workspace.execute(Command::Redo).unwrap();
        assert!(workspace.document.layer(1).unwrap().shape_fill.is_some());
    }
}
