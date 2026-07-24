use super::*;

const LASSO_SAMPLE_SCREEN_DISTANCE: f32 = 2.0;

fn clamped_canvas_point(geometry: CanvasGeometry, screen: Pos2) -> Pos2 {
    let point = geometry.screen_to_canvas(screen);
    Pos2::new(
        point
            .x
            .clamp(0.0, geometry.canvas.width() / geometry.pixels_per_point),
        point
            .y
            .clamp(0.0, geometry.canvas.height() / geometry.pixels_per_point),
    )
}

fn gesture_mode(
    input: &egui::InputState,
    configured: prism_core::SelectionCombineMode,
) -> prism_core::SelectionCombineMode {
    match (input.modifiers.shift, input.modifiers.alt) {
        (true, true) => prism_core::SelectionCombineMode::Intersect,
        (true, false) => prism_core::SelectionCombineMode::Add,
        (false, true) => prism_core::SelectionCombineMode::Subtract,
        (false, false) => configured,
    }
}

fn should_sample(points: &[Pos2], point: Pos2, geometry: CanvasGeometry) -> bool {
    points.last().is_none_or(|last| {
        geometry
            .canvas_to_screen(*last)
            .distance(geometry.canvas_to_screen(point))
            >= LASSO_SAMPLE_SCREEN_DISTANCE
    })
}

pub(super) fn paint_lasso_draft(ui: &egui::Ui, geometry: CanvasGeometry, points: &[Pos2]) {
    if points.len() < 2 {
        return;
    }
    let screen: Vec<_> = points
        .iter()
        .map(|point| geometry.canvas_to_screen(*point))
        .collect();
    ui.painter().add(egui::Shape::line(
        screen.clone(),
        Stroke::new(3.0, Color32::from_black_alpha(210)),
    ));
    ui.painter()
        .add(egui::Shape::line(screen, Stroke::new(1.0, Color32::WHITE)));
}

fn finalize_lasso(
    points: Vec<Pos2>,
    mode: prism_core::SelectionCombineMode,
    antialias: bool,
    overflowed: bool,
) -> anyhow::Result<Option<Command>> {
    if overflowed {
        anyhow::bail!(
            "Lasso exceeded the {}-sample gesture limit; draw a shorter path",
            prism_core::MAX_LASSO_INPUT_POINTS
        );
    }
    if points.len() < 3 {
        return Ok(None);
    }
    let points = points
        .into_iter()
        .map(|point| prism_core::LassoPoint::from_canvas(point.x, point.y))
        .collect::<anyhow::Result<Vec<_>>>()?;
    Ok(Some(Command::LassoSelection {
        points: prism_core::LassoPath::new(points)?,
        mode,
        antialias,
    }))
}

impl PrismApp {
    pub(super) fn cancel_lasso(&mut self) {
        self.selection_ui.lasso_points.clear();
        self.selection_ui.lasso_gesture_mode = None;
        self.selection_ui.lasso_overflowed = false;
    }

    pub(super) fn lasso_canvas_interaction(
        &mut self,
        ui: &mut egui::Ui,
        response: &egui::Response,
        geometry: CanvasGeometry,
    ) {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
        let pointer = ui.input(|input| input.pointer.interact_pos());
        if response.drag_started()
            && let Some(pointer) = pointer
            && geometry.canvas.contains(pointer)
        {
            self.cancel_lasso();
            self.selection_ui
                .lasso_points
                .push(clamped_canvas_point(geometry, pointer));
            self.selection_ui.lasso_gesture_mode =
                Some(ui.input(|input| gesture_mode(input, self.selection_ui.lasso_mode)));
        }
        if response.dragged()
            && let Some(pointer) = pointer
        {
            let point = clamped_canvas_point(geometry, pointer);
            if self.selection_ui.lasso_points.len() < prism_core::MAX_LASSO_INPUT_POINTS
                && should_sample(&self.selection_ui.lasso_points, point, geometry)
            {
                self.selection_ui.lasso_points.push(point);
            } else if self.selection_ui.lasso_points.len() == prism_core::MAX_LASSO_INPUT_POINTS
                && should_sample(&self.selection_ui.lasso_points, point, geometry)
            {
                self.selection_ui.lasso_overflowed = true;
            }
        }
        if response.drag_stopped() {
            let points = std::mem::take(&mut self.selection_ui.lasso_points);
            let mode = self
                .selection_ui
                .lasso_gesture_mode
                .take()
                .unwrap_or(self.selection_ui.lasso_mode);
            let overflowed = std::mem::take(&mut self.selection_ui.lasso_overflowed);
            match finalize_lasso(points, mode, self.selection_ui.lasso_antialias, overflowed) {
                Ok(Some(command)) => {
                    self.execute(command);
                }
                Ok(None) => {}
                Err(error) => {
                    self.status = error.to_string();
                    self.status_error = true;
                }
            }
        } else if !ui.input(|input| input.pointer.primary_down())
            && !self.selection_ui.lasso_points.is_empty()
        {
            self.cancel_lasso();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sampling_is_screen_space_and_bounded_by_zoom() {
        let geometry = CanvasGeometry {
            viewport: Rect::EVERYTHING,
            canvas: Rect::from_min_size(Pos2::ZERO, Vec2::new(400.0, 300.0)),
            pixels_per_point: 2.0,
        };
        let points = vec![Pos2::new(10.0, 10.0)];
        assert!(!should_sample(&points, Pos2::new(10.5, 10.0), geometry));
        assert!(should_sample(&points, Pos2::new(11.0, 10.0), geometry));
    }

    #[test]
    fn draft_projection_is_zoom_independent_in_screen_space() {
        let points = [Pos2::new(10.0, 20.0), Pos2::new(18.0, 29.0)];
        for pixels_per_point in [0.25, 1.0, 8.0] {
            let geometry = CanvasGeometry {
                viewport: Rect::EVERYTHING,
                canvas: Rect::from_min_size(
                    Pos2::new(40.0, 60.0),
                    Vec2::new(400.0, 300.0) * pixels_per_point,
                ),
                pixels_per_point,
            };
            let screen: Vec<_> = points
                .iter()
                .map(|point| geometry.canvas_to_screen(*point))
                .collect();
            assert_eq!(
                geometry.screen_to_canvas(screen[0]),
                points[0],
                "draft points must round-trip at {pixels_per_point}x"
            );
            assert_eq!(
                geometry.screen_to_canvas(screen[1]),
                points[1],
                "draft points must round-trip at {pixels_per_point}x"
            );
        }
    }

    #[test]
    fn modifier_modes_are_fixed_from_gesture_start() {
        let mut input = egui::RawInput::default();
        input.modifiers.shift = true;
        let context = egui::Context::default();
        context.begin_pass(input);
        let mode =
            context.input(|input| gesture_mode(input, prism_core::SelectionCombineMode::Replace));
        let _ = context.end_pass();
        assert_eq!(mode, prism_core::SelectionCombineMode::Add);
    }

    #[test]
    fn finalization_is_noop_for_short_drafts_and_one_command_for_valid_release() {
        assert!(
            finalize_lasso(
                vec![Pos2::ZERO, Pos2::new(1.0, 1.0)],
                prism_core::SelectionCombineMode::Replace,
                true,
                false,
            )
            .unwrap()
            .is_none()
        );
        let command = finalize_lasso(
            vec![Pos2::ZERO, Pos2::new(8.0, 0.0), Pos2::new(0.0, 8.0)],
            prism_core::SelectionCombineMode::Add,
            true,
            false,
        )
        .unwrap()
        .unwrap();
        assert!(matches!(
            command,
            Command::LassoSelection {
                mode: prism_core::SelectionCombineMode::Add,
                ..
            }
        ));
        assert!(
            finalize_lasso(
                vec![Pos2::ZERO; prism_core::MAX_LASSO_INPUT_POINTS],
                prism_core::SelectionCombineMode::Replace,
                true,
                true,
            )
            .is_err()
        );
    }
}
