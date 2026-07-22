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
}

struct BrushGesture {
    layer_id: Option<u64>,
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
        }
    }
}

impl PrismApp {
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
        self.composite_preview.reset();
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
        self.brush.gesture = Some(BrushGesture {
            layer_id,
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
                self.brush.preview = Some(preview);
                self.composite_preview.reset();
            }
            Err(error) => self.brush_error(error),
        }
    }

    fn finish_brush_gesture(&mut self) {
        let command = self.current_brush_command();
        self.brush.gesture = None;
        self.brush.preview = None;
        self.composite_preview.reset();
        match command {
            Ok(command) => {
                self.execute(command);
            }
            Err(error) => self.brush_error(error),
        }
    }

    fn brush_error(&mut self, error: anyhow::Error) {
        self.status = format!("{error:#}");
        self.status_error = true;
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
}
