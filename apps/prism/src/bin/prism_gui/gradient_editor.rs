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
            let edit = stop_color_editor(ui, (layer.id, index), &mut gradient.stops[index].color);
            widget_gradient(app, &edit.precision_response, layer.id, &gradient);
            visual_stop_color_gradient(app, &edit, layer.id, &gradient);
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

fn stop_color_editor(
    ui: &mut egui::Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    color: &mut [u8; 4],
) -> StopColorEditorResponse {
    let mut combined = straight_srgba_swatch(ui, *color);
    let anchor = combined.clone();
    let popup_id = ui.make_persistent_id(("gradient-stop-color", &id));
    let session_id = ui.make_persistent_id(("gradient-stop-color-session", &id));
    let visual_state_id = ui.make_persistent_id(("gradient-stop-color-hsva", &id));
    let mut visual_changed = false;
    let popup = egui::Popup::menu(&anchor)
        .id(popup_id)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            ui.set_min_width(300.0);
            ui.spacing_mut().slider_width = 275.0;
            ui.label(RichText::new("Visual color").size(9.0).color(MUTED));
            let cached = ui
                .ctx()
                .data(|data| data.get_temp::<VisualStopColorState>(visual_state_id));
            let mut visual_state = synchronized_visual_stop_color(cached, *color);
            visual_changed = egui::color_picker::color_picker_hsva_2d(
                ui,
                &mut visual_state.hsva,
                egui::color_picker::Alpha::OnlyBlend,
            );
            if visual_changed {
                *color = visual_state.hsva.to_srgba_unmultiplied();
                visual_state.srgba = *color;
            }
            ui.separator();
            ui.label(RichText::new("Straight sRGBA · U8").size(9.0).color(MUTED));
            for (channel, label) in ["Red", "Green", "Blue", "Alpha"].into_iter().enumerate() {
                let mut value = color[channel];
                let response = ui.add(egui::Slider::new(&mut value, 0..=255).text(label));
                if response.changed() {
                    set_stop_color_channel(color, channel, value);
                    visual_state.synchronize_exact_channel(*color, channel);
                }
                combined = combined.union(response);
            }
            ui.ctx()
                .data_mut(|data| data.insert_temp(visual_state_id, visual_state));
        });
    StopColorEditorResponse {
        precision_response: combined,
        session_id,
        visual_changed,
        popup_open: popup.is_some(),
    }
}

struct StopColorEditorResponse {
    precision_response: egui::Response,
    session_id: egui::Id,
    visual_changed: bool,
    popup_open: bool,
}

#[derive(Clone, Copy, Debug)]
struct VisualStopColorState {
    hsva: egui::ecolor::Hsva,
    srgba: [u8; 4],
}

fn synchronized_visual_stop_color(
    cached: Option<VisualStopColorState>,
    srgba: [u8; 4],
) -> VisualStopColorState {
    cached
        .filter(|state| state.srgba == srgba)
        .unwrap_or_else(|| VisualStopColorState {
            hsva: hsva_from_straight_srgba(srgba),
            srgba,
        })
}

fn hsva_from_straight_srgba([red, green, blue, alpha]: [u8; 4]) -> egui::ecolor::Hsva {
    egui::ecolor::Hsva::from_rgba_unmultiplied(
        egui::ecolor::linear_f32_from_gamma_u8(red),
        egui::ecolor::linear_f32_from_gamma_u8(green),
        egui::ecolor::linear_f32_from_gamma_u8(blue),
        egui::ecolor::linear_f32_from_linear_u8(alpha),
    )
}

impl VisualStopColorState {
    fn synchronize_exact_channel(&mut self, srgba: [u8; 4], channel: usize) {
        if channel == 3 {
            self.hsva.a = f32::from(srgba[3]) / 255.0;
        } else {
            self.hsva = hsva_from_straight_srgba(srgba);
        }
        self.srgba = srgba;
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct VisualColorInteractionPhases {
    begin: bool,
    apply: bool,
    finish: bool,
    clear: bool,
}

fn visual_color_interaction_phases(
    editor_active: bool,
    workspace_active: bool,
    changed: bool,
    finish_requested: bool,
) -> VisualColorInteractionPhases {
    if editor_active && !workspace_active {
        return VisualColorInteractionPhases {
            clear: true,
            ..Default::default()
        };
    }
    let begin = changed && !editor_active;
    VisualColorInteractionPhases {
        begin,
        apply: changed,
        finish: (editor_active || begin) && finish_requested,
        clear: false,
    }
}

fn visual_stop_color_gradient(
    app: &mut PrismApp,
    edit: &StopColorEditorResponse,
    id: u64,
    gradient: &ShapeGradient,
) {
    let context = &edit.precision_response.ctx;
    let editor_active =
        context.data(|data| data.get_temp::<bool>(edit.session_id).unwrap_or_default());
    let finish_requested = !edit.popup_open
        || context.input(|input| {
            input.pointer.any_released()
                || input.key_pressed(egui::Key::Enter)
                || input.key_pressed(egui::Key::Tab)
        });
    let phases = visual_color_interaction_phases(
        editor_active,
        app.workspace.interaction_active(),
        edit.visual_changed,
        finish_requested,
    );
    if phases.clear {
        context.data_mut(|data| data.remove::<bool>(edit.session_id));
        return;
    }
    let mut active = editor_active;
    if phases.begin {
        if app.workspace.interaction_active() {
            app.finish_interaction();
        }
        active = app.begin_workspace_interaction();
        if active {
            context.data_mut(|data| data.insert_temp(edit.session_id, true));
        }
    }
    if phases.apply && active {
        app.preview_command(Command::SetShapeFill {
            id,
            fill: Some(ShapeFill::Gradient(gradient.clone())),
        });
    }
    if phases.finish && active {
        app.finish_interaction();
        context.data_mut(|data| data.remove::<bool>(edit.session_id));
    }
}

fn straight_srgba_swatch(ui: &mut egui::Ui, color: [u8; 4]) -> egui::Response {
    let size = egui::vec2(24.0, 16.0);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            ui.is_enabled(),
            format!(
                "Edit gradient stop color: red {}, green {}, blue {}, alpha {}",
                color[0], color[1], color[2], color[3]
            ),
        )
    });
    if ui.is_rect_visible(rect) {
        let half = rect.size() * 0.5;
        for row in 0..2 {
            for column in 0..2 {
                let min = rect.min + egui::vec2(column as f32 * half.x, row as f32 * half.y);
                let cell = egui::Rect::from_min_size(min, half);
                let shade = if (row + column) % 2 == 0 { 72 } else { 112 };
                ui.painter()
                    .rect_filled(cell, 0.0, egui::Color32::from_gray(shade));
            }
        }
        ui.painter().rect_filled(
            rect,
            2.0,
            egui::Color32::from_rgba_unmultiplied(color[0], color[1], color[2], color[3]),
        );
        ui.painter().rect_stroke(
            rect,
            2.0,
            egui::Stroke::new(1.0, BORDER),
            egui::StrokeKind::Inside,
        );
    }
    response.on_hover_text(format!(
        "Straight sRGBA · R {} · G {} · B {} · A {}",
        color[0], color[1], color[2], color[3]
    ))
}

fn set_stop_color_channel(color: &mut [u8; 4], channel: usize, value: u8) {
    color[channel] = value;
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
    fn straight_srgba_channel_edits_never_round_unedited_bytes() {
        let mut color = [68, 180, 211, 180];
        set_stop_color_channel(&mut color, 3, 128);
        assert_eq!(color, [68, 180, 211, 128]);
        set_stop_color_channel(&mut color, 1, 181);
        assert_eq!(color, [68, 181, 211, 128]);
    }

    #[test]
    fn visual_picker_alpha_edits_preserve_straight_rgb_bytes() {
        let color = [68, 180, 211, 180];
        let mut visual = hsva_from_straight_srgba(color);
        visual.a = 128.0 / 255.0;
        assert_eq!(visual.to_srgba_unmultiplied(), [68, 180, 211, 128]);
    }

    #[test]
    fn visual_picker_preserves_hidden_straight_rgb_at_zero_and_low_alpha() {
        for alpha in [0, 1, 2, 8] {
            let color = [17, 33, 91, alpha];
            let state = synchronized_visual_stop_color(None, color);
            assert_eq!(state.srgba, color);
            assert_eq!(
                state.hsva.to_srgba_unmultiplied(),
                color,
                "opening the popup must not premultiply hidden RGB at alpha {alpha}"
            );
        }
    }

    #[test]
    fn transparent_exact_alpha_then_spectrum_edit_keeps_the_hidden_color_basis() {
        let transparent = [17, 33, 91, 0];
        let mut state = synchronized_visual_stop_color(None, transparent);
        assert!(state.hsva.v > 0.0, "transparent blue must not become black");

        state.synchronize_exact_channel([17, 33, 91, 96], 3);
        assert_eq!(state.hsva.to_srgba_unmultiplied(), [17, 33, 91, 96]);

        state.hsva.h = (state.hsva.h + 0.125).fract();
        let spectrum_edit = state.hsva.to_srgba_unmultiplied();
        assert_eq!(spectrum_edit[3], 96);
        assert_ne!(&spectrum_edit[..3], &[0, 0, 0]);
        assert_eq!(spectrum_edit.iter().take(3).copied().max(), Some(91));
    }

    #[test]
    fn visual_picker_retains_gray_hue_and_syncs_exact_alpha_without_resetting_it() {
        let gray = [128, 128, 128, 255];
        let mut cached = synchronized_visual_stop_color(None, gray);
        cached.hsva.h = 0.72;
        let mut retained = synchronized_visual_stop_color(Some(cached), gray);
        assert_eq!(retained.hsva.h, 0.72);

        retained.synchronize_exact_channel([128, 128, 128, 96], 3);
        assert_eq!(retained.hsva.h, 0.72);
        assert_eq!(retained.hsva.a, 96.0 / 255.0);

        retained.hsva.s = 0.8;
        let saturated = retained.hsva.to_srgba_unmultiplied();
        assert!(
            saturated[2] > saturated[0] && saturated[0] > saturated[1],
            "raising saturation must use the retained blue-magenta hue: {saturated:?}"
        );
    }

    #[test]
    fn visual_picker_drag_is_previewed_once_then_committed_or_cleared() {
        assert_eq!(
            visual_color_interaction_phases(false, false, true, false),
            VisualColorInteractionPhases {
                begin: true,
                apply: true,
                ..Default::default()
            }
        );
        assert_eq!(
            visual_color_interaction_phases(true, true, true, false),
            VisualColorInteractionPhases {
                apply: true,
                ..Default::default()
            }
        );
        assert_eq!(
            visual_color_interaction_phases(true, true, false, true),
            VisualColorInteractionPhases {
                finish: true,
                ..Default::default()
            }
        );
        assert_eq!(
            visual_color_interaction_phases(true, false, false, false),
            VisualColorInteractionPhases {
                clear: true,
                ..Default::default()
            }
        );
    }

    #[test]
    fn visual_stop_preview_is_one_undoable_revision_and_cancel_restores_bytes() {
        let mut workspace =
            prism_core::Workspace::new(prism_core::Document::new("Gradient color", 32, 24), None);
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
        let original = ShapeGradient::default();
        workspace
            .execute(Command::SetShapeFill {
                id: 1,
                fill: Some(ShapeFill::Gradient(original.clone())),
            })
            .unwrap();

        workspace.begin_interaction().unwrap();
        for color in [[220, 40, 80, 255], [30, 180, 210, 160]] {
            let mut preview = original.clone();
            preview.stops[0].color = color;
            workspace
                .preview(Command::SetShapeFill {
                    id: 1,
                    fill: Some(ShapeFill::Gradient(preview)),
                })
                .unwrap();
        }
        assert!(workspace.commit_interaction().unwrap());
        workspace.execute(Command::Undo).unwrap();
        assert_eq!(
            workspace.document.layer(1).unwrap().shape_fill,
            Some(ShapeFill::Gradient(original.clone()))
        );
        workspace.execute(Command::Redo).unwrap();

        let committed = workspace.document.layer(1).unwrap().shape_fill.clone();
        workspace.begin_interaction().unwrap();
        let mut canceled = original;
        canceled.stops[0].color = [1, 2, 3, 4];
        workspace
            .preview(Command::SetShapeFill {
                id: 1,
                fill: Some(ShapeFill::Gradient(canceled)),
            })
            .unwrap();
        assert!(workspace.cancel_interaction());
        assert_eq!(workspace.document.layer(1).unwrap().shape_fill, committed);
        workspace.execute(Command::Undo).unwrap();
        assert_eq!(
            workspace.document.layer(1).unwrap().shape_fill,
            Some(ShapeFill::Gradient(ShapeGradient::default()))
        );
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
