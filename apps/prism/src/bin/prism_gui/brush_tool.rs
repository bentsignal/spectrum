use super::*;
use anyhow::Context;

#[derive(Default)]
pub(super) struct BrushState {
    pub size: f32,
    pub hardness: f32,
    pub opacity: f32,
    pub spacing: f32,
    pub color: [u8; 4],
    gesture: Option<BrushGesture>,
    pub preview: Option<Document>,
    preview_key: Option<ProgressiveBrushPreview>,
    settling_layer_id: Option<u64>,
    next_gesture_id: u64,
    color_draft: Option<[u8; 4]>,
    color_picker_open: bool,
}

struct BrushGesture {
    layer_id: Option<u64>,
    target_layer_id: u64,
    gesture_id: u64,
    viewport: (u32, u32),
    transform: Transform,
    samples: Vec<prism_core::BrushSample>,
    last_raw_local: Pos2,
}

impl BrushState {
    pub(super) fn configured() -> Self {
        Self {
            size: 32.0,
            hardness: 0.8,
            opacity: 1.0,
            spacing: 0.15,
            color: [255; 4],
            gesture: None,
            preview: None,
            preview_key: None,
            settling_layer_id: None,
            next_gesture_id: 1,
            color_draft: None,
            color_picker_open: false,
        }
    }
}

impl PrismApp {
    pub(super) fn layer_visual_is_current(&self, id: u64, display_scale: f32) -> bool {
        let Ok(layer) = self.workspace.document.layer(id) else {
            return false;
        };
        !self.layer_visual_dirty.contains(&id)
            && self
                .layer_visuals
                .get(&id)
                .is_some_and(|entry| entry.key == LayerVisualKey::new(layer, display_scale))
    }

    pub(super) fn brush_canvas_interaction(
        &mut self,
        ui: &mut egui::Ui,
        response: &egui::Response,
        geometry: CanvasGeometry,
    ) {
        if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
        }
        let pointer = response.interact_pointer_pos();
        if response.drag_started()
            && let Some(pointer) = pointer
        {
            let press = ui
                .input(|input| input.pointer.press_origin())
                .unwrap_or(pointer);
            if geometry.canvas.contains(press) {
                self.begin_brush_gesture(geometry.screen_to_canvas(press));
            }
        }
        if response.dragged()
            && let Some(pointer) = pointer
            && self.brush.gesture.is_some()
        {
            self.push_brush_sample(geometry.screen_to_canvas(pointer));
            self.refresh_brush_preview();
        }
        if response.drag_stopped() && self.brush.gesture.is_some() {
            self.finish_brush_gesture();
        } else if response.clicked()
            && let Some(pointer) = pointer
            && geometry.canvas.contains(pointer)
        {
            self.begin_brush_gesture(geometry.screen_to_canvas(pointer));
            if self.brush.gesture.is_some() {
                self.finish_brush_gesture();
            }
        }
    }

    pub(super) fn cancel_brush(&mut self) {
        self.brush.gesture = None;
        self.brush.preview = None;
        self.brush.preview_key = None;
        self.brush.settling_layer_id = None;
        self.composite_preview.reset();
    }

    pub(super) fn brush_preview_key(&self) -> Option<ProgressiveBrushPreview> {
        self.brush.preview_key
    }

    pub(super) fn settle_brush_preview_if_ready(&mut self, display_scale: f32) {
        let Some(id) = self.brush.settling_layer_id else {
            return;
        };
        if self.layer_visual_is_current(id, display_scale) {
            self.brush.preview = None;
            self.brush.preview_key = None;
            self.brush.settling_layer_id = None;
            self.composite_preview.reset();
        }
    }

    pub(super) fn brush_settings_control(&mut self, ui: &mut egui::Ui) {
        let popup_id = ui.make_persistent_id("brush-settings-popup");
        let open = egui::Popup::is_id_open(ui.ctx(), popup_id);
        let button = ui.add(
            egui::Button::new(format!("Brush Settings · {:.0}px", self.brush.size)).selected(open),
        );
        let popup = egui::Popup::menu(&button)
            .id(popup_id)
            .close_behavior(brush_popup_close_behavior())
            .width(260.0)
            .show(|ui| self.brush_settings_contents(ui));
        if popup.is_none() {
            self.brush.color_draft = None;
            self.brush.color_picker_open = false;
        }
    }

    pub(super) fn cancel_brush_color_picker(&mut self, _context: &egui::Context) -> bool {
        if !self.brush.color_picker_open {
            return false;
        }
        self.brush.color_draft = None;
        self.brush.color_picker_open = false;
        true
    }

    pub(super) fn brush_color_picker_open(&self) -> bool {
        self.brush.color_picker_open
    }

    fn brush_settings_contents(&mut self, ui: &mut egui::Ui) {
        ui.set_min_width(240.0);
        ui.add(
            egui::Slider::new(&mut self.brush.size, 1.0..=512.0)
                .text("Size")
                .logarithmic(true),
        );
        ui.add(egui::Slider::new(&mut self.brush.hardness, 0.0..=1.0).text("Hardness"));
        ui.add(egui::Slider::new(&mut self.brush.opacity, 0.0..=1.0).text("Opacity"));
        ui.add(egui::Slider::new(&mut self.brush.spacing, 0.01..=1.0).text("Spacing"));
        if self.tool != Tool::Brush {
            return;
        }

        let committed = Color32::from_rgba_unmultiplied(
            self.brush.color[0],
            self.brush.color[1],
            self.brush.color[2],
            self.brush.color[3],
        );
        let button = ui.horizontal(|ui| {
            ui.label("Color");
            ui.add(
                egui::Button::new("Fill")
                    .fill(committed)
                    .stroke(Stroke::new(1.0, TEXT))
                    .selected(self.brush.color_picker_open),
            )
        });
        if button.inner.clicked() {
            if self.brush.color_picker_open {
                self.brush.color_draft = None;
                self.brush.color_picker_open = false;
            } else {
                self.brush.color_draft = Some(self.brush.color);
                self.brush.color_picker_open = true;
            }
        }
        if self.brush.color_picker_open {
            let action = egui::Frame::group(ui.style())
                .show(ui, |ui| {
                    ui.set_min_width(280.0);
                    let draft = self.brush.color_draft.get_or_insert(self.brush.color);
                    let mut color =
                        Color32::from_rgba_unmultiplied(draft[0], draft[1], draft[2], draft[3]);
                    egui::color_picker::color_picker_color32(
                        ui,
                        &mut color,
                        egui::color_picker::Alpha::OnlyBlend,
                    );
                    *draft = color.to_array();
                    ui.separator();
                    let keyboard_cancel = ui.input_mut(|input| {
                        input.consume_key(egui::Modifiers::NONE, egui::Key::Escape)
                    });
                    let mut cancel_clicked = false;
                    let mut apply_clicked = false;
                    ui.horizontal(|ui| {
                        if ui.button("Cancel").clicked() {
                            cancel_clicked = true;
                        }
                        if ui.button("Apply").clicked() {
                            apply_clicked = true;
                        }
                    });
                    brush_color_action(cancel_clicked, apply_clicked, keyboard_cancel)
                })
                .inner;
            if let Some(action) = action {
                match action {
                    BrushColorAction::Apply => {
                        if let Some(color) = self.brush.color_draft.take() {
                            self.brush.color = color;
                        }
                    }
                    BrushColorAction::Cancel => {
                        self.brush.color_draft = None;
                    }
                }
                self.brush.color_picker_open = false;
            }
        }
    }

    fn begin_brush_gesture(&mut self, canvas: Pos2) {
        let selected = self.selected_layer().cloned();
        if let Some(layer) = selected.as_ref()
            && matches!(layer.kind, LayerKind::Paint { .. })
        {
            if layer.locked {
                self.status = "Unlock the focused Paint layer before painting".into();
                self.status_error = true;
                return;
            }
            if !prism_core::paint_layer_allows_direct_strokes(layer) {
                self.status =
                    "Reset geometric adjustments before painting; reapply them afterward".into();
                self.status_error = true;
                return;
            }
        }
        let target = selected.as_ref().and_then(|layer| match &layer.kind {
            LayerKind::Paint { program } => Some((
                Some(layer.id),
                (program.width, program.height),
                layer.transform,
            )),
            _ => None,
        });
        if self.tool == Tool::Eraser && target.is_none() {
            self.status = "Select an unlocked Paint layer before erasing".into();
            self.status_error = true;
            return;
        }
        let (layer_id, viewport, transform) = target.unwrap_or((
            None,
            (
                self.workspace.document.width,
                self.workspace.document.height,
            ),
            Transform::default(),
        ));
        let local = canvas_to_paint_local(canvas, viewport, transform);
        if !paint_viewport_contains(local, viewport) {
            self.status = "Start the stroke inside the focused Paint layer".into();
            self.status_error = true;
            return;
        }
        self.brush.preview = None;
        self.brush.preview_key = None;
        self.brush.settling_layer_id = None;
        self.composite_preview.reset();
        let gesture_id = self.brush.next_gesture_id;
        self.brush.next_gesture_id = self.brush.next_gesture_id.wrapping_add(1).max(1);
        self.brush.gesture = Some(BrushGesture {
            layer_id,
            target_layer_id: layer_id.unwrap_or(self.workspace.document.next_id),
            gesture_id,
            viewport,
            transform,
            samples: vec![brush_sample(local)],
            last_raw_local: local,
        });
        self.refresh_brush_preview();
    }

    fn push_brush_sample(&mut self, canvas: Pos2) {
        let Some(gesture) = self.brush.gesture.as_mut() else {
            return;
        };
        if gesture.samples.len() >= prism_core::MAX_BRUSH_SAMPLES_PER_STROKE {
            return;
        }
        let local = canvas_to_paint_local(canvas, gesture.viewport, gesture.transform);
        let clipped = clip_segment_to_viewport(gesture.last_raw_local, local, gesture.viewport);
        gesture.last_raw_local = local;
        if let Some((start, end)) = clipped {
            for point in [start, end] {
                if gesture.samples.len() >= prism_core::MAX_BRUSH_SAMPLES_PER_STROKE {
                    break;
                }
                let sample = brush_sample(point);
                if gesture
                    .samples
                    .last()
                    .is_none_or(|last| (last.x - sample.x).hypot(last.y - sample.y) >= 0.25)
                {
                    gesture.samples.push(sample);
                }
            }
        }
    }

    fn current_brush_command(&self) -> anyhow::Result<Command> {
        let gesture = self
            .brush
            .gesture
            .as_ref()
            .context("no Brush gesture is active")?;
        brush_command(self.tool, &self.brush, gesture)
    }

    fn refresh_brush_preview(&mut self) {
        let Ok(command) = self.current_brush_command() else {
            return;
        };
        match prism_core::preview_paint_command(&self.workspace.document, command) {
            Ok(preview) => {
                let gesture = self.brush.gesture.as_ref().expect("Brush gesture exists");
                self.brush.preview_key = Some(ProgressiveBrushPreview {
                    gesture_id: gesture.gesture_id,
                    target_layer_id: gesture.target_layer_id,
                    sample_count: gesture.samples.len(),
                    mode: if self.tool == Tool::Eraser {
                        prism_core::BrushMode::Erase
                    } else {
                        prism_core::BrushMode::Paint
                    },
                });
                self.brush.preview = Some(preview);
            }
            Err(error) => self.brush_error(error),
        }
    }

    fn finish_brush_gesture(&mut self) {
        let command = self.current_brush_command();
        let target_layer_id = self
            .brush
            .gesture
            .as_ref()
            .map(|gesture| gesture.target_layer_id);
        self.brush.gesture = None;
        match command {
            Ok(command) => {
                if self.execute(command) {
                    self.brush.settling_layer_id = target_layer_id;
                } else {
                    self.cancel_brush();
                }
            }
            Err(error) => {
                self.cancel_brush();
                self.brush_error(error);
            }
        }
    }

    fn brush_error(&mut self, error: anyhow::Error) {
        self.status = format!("{error:#}");
        self.status_error = true;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BrushColorAction {
    Apply,
    Cancel,
}

fn brush_popup_close_behavior() -> egui::PopupCloseBehavior {
    egui::PopupCloseBehavior::CloseOnClickOutside
}

fn brush_color_action(
    cancel_clicked: bool,
    apply_clicked: bool,
    escape_pressed: bool,
) -> Option<BrushColorAction> {
    if cancel_clicked || escape_pressed {
        Some(BrushColorAction::Cancel)
    } else if apply_clicked {
        Some(BrushColorAction::Apply)
    } else {
        None
    }
}

fn brush_command(
    tool: Tool,
    brush: &BrushState,
    gesture: &BrushGesture,
) -> anyhow::Result<Command> {
    let stroke = prism_core::BrushStroke::new(
        prism_core::BrushStyle {
            mode: if tool == Tool::Eraser {
                prism_core::BrushMode::Erase
            } else {
                prism_core::BrushMode::Paint
            },
            color: brush.color,
            size: brush.size,
            hardness: brush.hardness,
            opacity: brush.opacity,
            spacing: brush.spacing,
        },
        gesture.samples.clone(),
    )?;
    Ok(if let Some(id) = gesture.layer_id {
        Command::AddBrushStroke {
            id,
            stroke,
            selection: prism_core::PaintSelection::Current,
        }
    } else {
        Command::AddPaintLayerWithStroke {
            name: None,
            width: gesture.viewport.0,
            height: gesture.viewport.1,
            stroke,
            selection: prism_core::PaintSelection::Current,
        }
    })
}

fn brush_sample(local: Pos2) -> prism_core::BrushSample {
    prism_core::BrushSample {
        x: local.x,
        y: local.y,
        pressure: 1.0,
    }
}

fn paint_viewport_contains(point: Pos2, viewport: (u32, u32)) -> bool {
    point.x >= 0.0 && point.y >= 0.0 && point.x <= viewport.0 as f32 && point.y <= viewport.1 as f32
}

fn clip_segment_to_viewport(start: Pos2, end: Pos2, viewport: (u32, u32)) -> Option<(Pos2, Pos2)> {
    let delta = end - start;
    let mut enter = 0.0_f32;
    let mut exit = 1.0_f32;
    for (p, q) in [
        (-delta.x, start.x),
        (delta.x, viewport.0 as f32 - start.x),
        (-delta.y, start.y),
        (delta.y, viewport.1 as f32 - start.y),
    ] {
        if p.abs() <= f32::EPSILON {
            if q < 0.0 {
                return None;
            }
            continue;
        }
        let ratio = q / p;
        if p < 0.0 {
            enter = enter.max(ratio);
        } else {
            exit = exit.min(ratio);
        }
        if enter > exit {
            return None;
        }
    }
    Some((start + delta * enter, start + delta * exit))
}

fn canvas_to_paint_local(canvas: Pos2, viewport: (u32, u32), transform: Transform) -> Pos2 {
    let center = Vec2::new(
        viewport.0 as f32 * transform.scale_x * 0.5,
        viewport.1 as f32 * transform.scale_y * 0.5,
    );
    let point = canvas - Pos2::new(transform.x, transform.y) - center;
    let radians = -transform.rotation.to_radians();
    let (sin, cos) = radians.sin_cos();
    Pos2::new(
        (point.x * cos - point.y * sin + center.x) / transform.scale_x,
        (point.x * sin + point.y * cos + center.y) / transform.scale_y,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gesture(layer_id: Option<u64>) -> BrushGesture {
        BrushGesture {
            layer_id,
            target_layer_id: layer_id.unwrap_or(1),
            gesture_id: 7,
            viewport: (100, 80),
            transform: Transform::default(),
            samples: vec![brush_sample(Pos2::new(12.5, 14.5))],
            last_raw_local: Pos2::new(12.5, 14.5),
        }
    }

    #[test]
    fn inverse_mapping_handles_centered_rotation_and_nonuniform_scale() {
        let viewport = (120, 80);
        let transform = Transform {
            x: 37.0,
            y: 21.0,
            scale_x: 1.5,
            scale_y: 0.7,
            rotation: 31.0,
        };
        let local = Pos2::new(23.0, 61.0);
        let center = Vec2::new(
            viewport.0 as f32 * transform.scale_x * 0.5,
            viewport.1 as f32 * transform.scale_y * 0.5,
        );
        let scaled = Vec2::new(local.x * transform.scale_x, local.y * transform.scale_y);
        let delta = scaled - center;
        let (sin, cos) = transform.rotation.to_radians().sin_cos();
        let canvas = Pos2::new(
            transform.x + center.x + delta.x * cos - delta.y * sin,
            transform.y + center.y + delta.x * sin + delta.y * cos,
        );
        let mapped = canvas_to_paint_local(canvas, viewport, transform);
        assert!((mapped.x - local.x).abs() < 0.001);
        assert!((mapped.y - local.y).abs() < 0.001);
    }

    #[test]
    fn viewport_segment_clipping_does_not_smear_outside_drags_onto_edges() {
        let clipped =
            clip_segment_to_viewport(Pos2::new(50.0, 40.0), Pos2::new(150.0, 140.0), (100, 80))
                .unwrap();
        assert_eq!(clipped.0, Pos2::new(50.0, 40.0));
        assert_eq!(clipped.1, Pos2::new(90.0, 80.0));
        assert!(
            clip_segment_to_viewport(Pos2::new(-20.0, -10.0), Pos2::new(-2.0, 70.0), (100, 80))
                .is_none()
        );
    }

    #[test]
    fn one_gesture_builds_one_atomic_core_command() {
        let brush = BrushState::configured();
        assert!(matches!(
            brush_command(Tool::Brush, &brush, &gesture(None)).unwrap(),
            Command::AddPaintLayerWithStroke { .. }
        ));
        assert!(matches!(
            brush_command(Tool::Eraser, &brush, &gesture(Some(9))).unwrap(),
            Command::AddBrushStroke { id: 9, .. }
        ));
    }

    #[test]
    fn expanded_color_picker_keeps_settings_open_and_maps_pointer_and_keyboard_actions() {
        assert_eq!(
            brush_popup_close_behavior(),
            egui::PopupCloseBehavior::CloseOnClickOutside
        );
        assert_eq!(
            brush_color_action(false, true, false),
            Some(BrushColorAction::Apply)
        );
        assert_eq!(
            brush_color_action(true, false, false),
            Some(BrushColorAction::Cancel)
        );
        assert_eq!(
            brush_color_action(false, false, true),
            Some(BrushColorAction::Cancel)
        );
        assert_eq!(brush_color_action(false, false, false), None);
    }
}
