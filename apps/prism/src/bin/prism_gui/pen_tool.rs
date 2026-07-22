use super::*;

use prism_core::{PathAnchor, PathFillRule, PathGeometry, VectorMask};

const ANCHOR_RADIUS: f32 = 6.0;
const HANDLE_RADIUS: f32 = 5.0;

#[derive(Default)]
pub(super) struct PenState {
    draft: Option<PenDraft>,
    edit: Option<PenEdit>,
    copied_mask: Option<PathGeometry>,
}

struct PenDraft {
    anchors: Vec<PathAnchor>,
    dragging_anchor: Option<usize>,
}

#[derive(Clone, Copy)]
struct PenEdit {
    layer_id: u64,
    anchor_index: usize,
    part: AnchorPart,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AnchorPart {
    Point,
    Incoming,
    Outgoing,
}

impl PenState {
    pub(super) fn copied_mask(&self) -> Option<&PathGeometry> {
        self.copied_mask.as_ref()
    }

    pub(super) fn copy_mask(&mut self, geometry: PathGeometry) {
        self.copied_mask = Some(geometry);
    }
}

impl PrismApp {
    pub(super) fn cancel_pen(&mut self) {
        if let Some(edit) = self.pen.edit.take() {
            self.workspace.cancel_interaction();
            self.layer_visual_dirty.insert(edit.layer_id);
        }
        self.pen.draft = None;
    }

    pub(super) fn finish_pen_path(&mut self, closed: bool) -> bool {
        let Some(draft) = self.pen.draft.take() else {
            return false;
        };
        let minimum = if closed { 3 } else { 2 };
        if draft.anchors.len() < minimum {
            self.pen.draft = Some(draft);
            self.status = format!(
                "A {} path needs at least {minimum} anchors",
                if closed { "closed" } else { "open" }
            );
            self.status_error = true;
            return false;
        }
        let result =
            geometry_from_canvas_anchors(draft.anchors, closed).map(|(origin, geometry)| {
                Command::AddPath {
                    name: None,
                    geometry,
                    color: [236, 241, 248, 255],
                    x: origin.x,
                    y: origin.y,
                }
            });
        match result {
            Ok(command) => {
                if self.execute(command) {
                    self.tool = Tool::Pen;
                    true
                } else {
                    false
                }
            }
            Err(error) => {
                self.status = format!("Could not finish path: {error:#}");
                self.status_error = true;
                false
            }
        }
    }

    pub(super) fn pen_canvas_interaction(
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
            if !geometry.canvas.contains(press) {
                return;
            }
            if self.begin_pen_edit(geometry, press) {
                return;
            }
            let canvas = geometry.screen_to_canvas(press);
            let draft = self.pen.draft.get_or_insert_with(|| PenDraft {
                anchors: Vec::new(),
                dragging_anchor: None,
            });
            if draft.anchors.len() < prism_core::MAX_PATH_ANCHORS {
                draft.anchors.push(PathAnchor::corner(canvas.x, canvas.y));
                draft.dragging_anchor = Some(draft.anchors.len() - 1);
            }
        }
        if response.dragged()
            && let Some(pointer) = pointer
        {
            if self.pen.edit.is_some() {
                self.preview_pen_edit(geometry, pointer);
            } else if let Some(draft) = self.pen.draft.as_mut()
                && let Some(index) = draft.dragging_anchor
            {
                let current = geometry.screen_to_canvas(pointer);
                let anchor = &mut draft.anchors[index];
                let delta = current - Pos2::new(anchor.point[0], anchor.point[1]);
                anchor.handle_out = [delta.x, delta.y];
                anchor.handle_in = [-delta.x, -delta.y];
            }
        }
        if response.drag_stopped() {
            if self.pen.edit.take().is_some() {
                self.finish_interaction();
            }
            if let Some(draft) = self.pen.draft.as_mut() {
                draft.dragging_anchor = None;
            }
        } else if response.double_clicked() {
            self.finish_pen_path(false);
        } else if response.clicked()
            && let Some(pointer) = pointer
            && geometry.canvas.contains(pointer)
        {
            if self.draft_first_anchor_hit(geometry, pointer) {
                self.finish_pen_path(true);
            } else {
                let canvas = geometry.screen_to_canvas(pointer);
                let draft = self.pen.draft.get_or_insert_with(|| PenDraft {
                    anchors: Vec::new(),
                    dragging_anchor: None,
                });
                if draft.anchors.len() < prism_core::MAX_PATH_ANCHORS {
                    draft.anchors.push(PathAnchor::corner(canvas.x, canvas.y));
                }
            }
        }
    }

    fn begin_pen_edit(&mut self, geometry: CanvasGeometry, pointer: Pos2) -> bool {
        let Some(layer) = self.selected_layer().cloned() else {
            return false;
        };
        let LayerKind::Path { geometry: path, .. } = &layer.kind else {
            return false;
        };
        let Some((anchor_index, part)) = path_anchor_hit(geometry, &layer, path, pointer) else {
            return false;
        };
        if layer.locked {
            self.status = "Unlock the path before editing its anchors".into();
            self.status_error = true;
            return true;
        }
        self.pen.draft = None;
        self.workspace.begin_interaction();
        self.pen.edit = Some(PenEdit {
            layer_id: layer.id,
            anchor_index,
            part,
        });
        true
    }

    fn preview_pen_edit(&mut self, geometry: CanvasGeometry, pointer: Pos2) {
        let Some(edit) = self.pen.edit else {
            return;
        };
        let Ok(layer) = self.workspace.document.layer(edit.layer_id).cloned() else {
            return;
        };
        if layer.locked {
            return;
        }
        let LayerKind::Path { geometry: path, .. } = &layer.kind else {
            return;
        };
        let local = canvas_to_path_local(&layer, path, geometry.screen_to_canvas(pointer));
        let mut anchor = path.anchors()[edit.anchor_index];
        match edit.part {
            AnchorPart::Point => anchor.point = [local.x, local.y],
            AnchorPart::Incoming => {
                anchor.handle_in = [local.x - anchor.point[0], local.y - anchor.point[1]]
            }
            AnchorPart::Outgoing => {
                anchor.handle_out = [local.x - anchor.point[0], local.y - anchor.point[1]]
            }
        }
        let Ok((path, transform)) = reframe_path_edit(&layer, path, edit.anchor_index, anchor)
        else {
            return;
        };
        self.preview_commands(vec![
            Command::ReplacePath {
                id: edit.layer_id,
                geometry: path,
            },
            Command::SetTransform {
                id: edit.layer_id,
                transform,
            },
        ]);
    }

    fn draft_first_anchor_hit(&self, geometry: CanvasGeometry, pointer: Pos2) -> bool {
        self.pen.draft.as_ref().is_some_and(|draft| {
            draft.anchors.len() >= 3
                && geometry
                    .canvas_to_screen(Pos2::new(
                        draft.anchors[0].point[0],
                        draft.anchors[0].point[1],
                    ))
                    .distance(pointer)
                    <= ANCHOR_RADIUS * 1.8
        })
    }

    pub(super) fn paint_pen_overlay(&self, ui: &egui::Ui, geometry: CanvasGeometry) {
        if self.tool != Tool::Pen {
            return;
        }
        if let Some(draft) = &self.pen.draft {
            paint_path_segments(ui, &draft.anchors, false, |point| {
                geometry.canvas_to_screen(Pos2::new(point[0], point[1]))
            });
            paint_path_controls(ui, &draft.anchors, |point| {
                geometry.canvas_to_screen(Pos2::new(point[0], point[1]))
            });
            return;
        }
        let Some(layer) = self.selected_layer() else {
            return;
        };
        let LayerKind::Path { geometry: path, .. } = &layer.kind else {
            return;
        };
        paint_path_segments(ui, path.anchors(), path.closed(), |point| {
            geometry.canvas_to_screen(path_local_to_canvas(layer, path, point))
        });
        paint_path_controls(ui, path.anchors(), |point| {
            geometry.canvas_to_screen(path_local_to_canvas(layer, path, point))
        });
    }

    pub(super) fn copy_path_for_vector_mask(&mut self, geometry: PathGeometry) {
        match VectorMask::new(geometry.clone(), false) {
            Ok(_) => {
                self.pen.copy_mask(geometry);
                self.status = "Copied path geometry for reuse as a vector mask".into();
                self.status_error = false;
            }
            Err(error) => {
                self.status = format!("Cannot use this path as a vector mask: {error:#}");
                self.status_error = true;
            }
        }
    }
}

fn geometry_from_canvas_anchors(
    anchors: Vec<PathAnchor>,
    closed: bool,
) -> anyhow::Result<(Pos2, PathGeometry)> {
    let (minimum, maximum) = path_control_bounds(&anchors);
    let origin = Pos2::new(minimum.x.floor(), minimum.y.floor());
    let width = (maximum.x.ceil() - origin.x).max(1.0) as u32;
    let height = (maximum.y.ceil() - origin.y).max(1.0) as u32;
    let anchors = anchors
        .into_iter()
        .map(|mut anchor| {
            anchor.point[0] -= origin.x;
            anchor.point[1] -= origin.y;
            anchor
        })
        .collect::<Vec<_>>();
    Ok((
        origin,
        PathGeometry::new(width, height, closed, PathFillRule::EvenOdd, anchors)?,
    ))
}

fn reframe_path_edit(
    layer: &Layer,
    path: &PathGeometry,
    index: usize,
    anchor: PathAnchor,
) -> anyhow::Result<(PathGeometry, Transform)> {
    let mut anchors = path.anchors().to_vec();
    anchors[index] = anchor;
    let (minimum, maximum) = path_control_bounds(&anchors);
    let shift = Vec2::new(minimum.x.min(0.0).floor(), minimum.y.min(0.0).floor());
    let right = maximum.x.max(path.width() as f32).ceil();
    let bottom = maximum.y.max(path.height() as f32).ceil();
    let width = (right - shift.x).max(1.0) as u32;
    let height = (bottom - shift.y).max(1.0) as u32;
    for anchor in &mut anchors {
        anchor.point[0] -= shift.x;
        anchor.point[1] -= shift.y;
    }
    let geometry = PathGeometry::new(width, height, path.closed(), path.fill_rule(), anchors)?;
    let transform = compensated_transform(
        layer.transform,
        Vec2::new(path.width() as f32, path.height() as f32),
        Vec2::new(width as f32, height as f32),
        shift,
    );
    Ok((geometry, transform))
}

fn compensated_transform(
    mut transform: Transform,
    old_size: Vec2,
    new_size: Vec2,
    shift: Vec2,
) -> Transform {
    let scale =
        |vector: Vec2| Vec2::new(vector.x * transform.scale_x, vector.y * transform.scale_y);
    let rotate = |vector: Vec2| {
        let (sin, cos) = prism_core::rotation_sin_cos(transform.rotation);
        Vec2::new(
            vector.x * cos - vector.y * sin,
            vector.x * sin + vector.y * cos,
        )
    };
    let size_delta = scale((old_size - new_size) * 0.5);
    let delta = size_delta - rotate(size_delta) + rotate(scale(shift));
    transform.x += delta.x;
    transform.y += delta.y;
    transform
}

fn path_control_bounds(anchors: &[PathAnchor]) -> (Pos2, Pos2) {
    anchors.iter().fold(
        (
            Pos2::new(f32::INFINITY, f32::INFINITY),
            Pos2::new(f32::NEG_INFINITY, f32::NEG_INFINITY),
        ),
        |(mut minimum, mut maximum), anchor| {
            for point in [anchor.point, anchor.incoming(), anchor.outgoing()] {
                minimum.x = minimum.x.min(point[0]);
                minimum.y = minimum.y.min(point[1]);
                maximum.x = maximum.x.max(point[0]);
                maximum.y = maximum.y.max(point[1]);
            }
            (minimum, maximum)
        },
    )
}

fn path_local_to_canvas(layer: &Layer, path: &PathGeometry, point: [f32; 2]) -> Pos2 {
    let center = Vec2::new(path.width() as f32 * 0.5, path.height() as f32 * 0.5);
    let scaled = Vec2::new(
        (point[0] - center.x) * layer.transform.scale_x,
        (point[1] - center.y) * layer.transform.scale_y,
    );
    let (sin, cos) = prism_core::rotation_sin_cos(layer.transform.rotation);
    let rotated = Vec2::new(
        scaled.x * cos - scaled.y * sin,
        scaled.x * sin + scaled.y * cos,
    );
    Pos2::new(
        layer.transform.x + center.x * layer.transform.scale_x + rotated.x,
        layer.transform.y + center.y * layer.transform.scale_y + rotated.y,
    )
}

fn canvas_to_path_local(layer: &Layer, path: &PathGeometry, point: Pos2) -> Pos2 {
    let center = Vec2::new(path.width() as f32 * 0.5, path.height() as f32 * 0.5);
    let canvas_center = Pos2::new(
        layer.transform.x + center.x * layer.transform.scale_x,
        layer.transform.y + center.y * layer.transform.scale_y,
    );
    let vector = point - canvas_center;
    let (sin, cos) = prism_core::rotation_sin_cos(layer.transform.rotation);
    let unrotated = Vec2::new(
        vector.x * cos + vector.y * sin,
        -vector.x * sin + vector.y * cos,
    );
    Pos2::new(
        center.x + unrotated.x / layer.transform.scale_x,
        center.y + unrotated.y / layer.transform.scale_y,
    )
}

fn path_anchor_hit(
    canvas: CanvasGeometry,
    layer: &Layer,
    path: &PathGeometry,
    pointer: Pos2,
) -> Option<(usize, AnchorPart)> {
    for (index, anchor) in path.anchors().iter().enumerate() {
        for (part, point) in [
            (AnchorPart::Incoming, anchor.incoming()),
            (AnchorPart::Outgoing, anchor.outgoing()),
        ] {
            if point != anchor.point
                && canvas
                    .canvas_to_screen(path_local_to_canvas(layer, path, point))
                    .distance(pointer)
                    <= HANDLE_RADIUS * 1.8
            {
                return Some((index, part));
            }
        }
    }
    path.anchors()
        .iter()
        .enumerate()
        .find_map(|(index, anchor)| {
            (canvas
                .canvas_to_screen(path_local_to_canvas(layer, path, anchor.point))
                .distance(pointer)
                <= ANCHOR_RADIUS * 1.8)
                .then_some((index, AnchorPart::Point))
        })
}

fn paint_path_segments(
    ui: &egui::Ui,
    anchors: &[PathAnchor],
    closed: bool,
    map: impl Fn([f32; 2]) -> Pos2,
) {
    if anchors.len() < 2 {
        return;
    }
    let segments = if closed {
        anchors.len()
    } else {
        anchors.len() - 1
    };
    for index in 0..segments {
        let start = anchors[index];
        let end = anchors[(index + 1) % anchors.len()];
        let mut points = Vec::with_capacity(25);
        for step in 0..=24 {
            let t = step as f32 / 24.0;
            points.push(map(cubic_point(
                start.point,
                start.outgoing(),
                end.incoming(),
                end.point,
                t,
            )));
        }
        ui.painter()
            .add(egui::Shape::line(points, Stroke::new(1.5, ACCENT)));
    }
}

fn paint_path_controls(ui: &egui::Ui, anchors: &[PathAnchor], map: impl Fn([f32; 2]) -> Pos2) {
    for anchor in anchors {
        let point = map(anchor.point);
        for handle in [anchor.incoming(), anchor.outgoing()] {
            if handle != anchor.point {
                let handle = map(handle);
                ui.painter()
                    .line_segment([point, handle], Stroke::new(1.0, MUTED));
                ui.painter().circle_filled(handle, HANDLE_RADIUS, PANEL);
                ui.painter()
                    .circle_stroke(handle, HANDLE_RADIUS, Stroke::new(1.5, ACCENT));
            }
        }
        ui.painter().circle_filled(point, ANCHOR_RADIUS, ACCENT);
        ui.painter()
            .circle_stroke(point, ANCHOR_RADIUS, Stroke::new(1.5, TEXT));
    }
}

fn cubic_point(a: [f32; 2], b: [f32; 2], c: [f32; 2], d: [f32; 2], t: f32) -> [f32; 2] {
    let inverse = 1.0 - t;
    let weights = [
        inverse.powi(3),
        3.0 * inverse.powi(2) * t,
        3.0 * inverse * t * t,
        t.powi(3),
    ];
    [
        a[0] * weights[0] + b[0] * weights[1] + c[0] * weights[2] + d[0] * weights[3],
        a[1] * weights[0] + b[1] * weights[1] + c[1] * weights[2] + d[1] * weights[3],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path() -> PathGeometry {
        PathGeometry::new(
            100,
            80,
            false,
            PathFillRule::EvenOdd,
            vec![
                PathAnchor::corner(0.0, 0.0),
                PathAnchor::corner(100.0, 80.0),
            ],
        )
        .unwrap()
    }

    #[test]
    fn varied_zoom_canvas_mapping_round_trips_rotated_scaled_paths() {
        let layer = Layer {
            transform: Transform {
                x: 30.0,
                y: -12.0,
                scale_x: 1.7,
                scale_y: 0.6,
                rotation: 37.0,
            },
            kind: LayerKind::Path {
                geometry: path(),
                color: [255; 4],
            },
            ..Layer::default()
        };
        let LayerKind::Path { geometry, .. } = &layer.kind else {
            unreachable!()
        };
        for zoom in [0.1, 0.75, 1.0, 4.0, 16.0] {
            let canvas = CanvasGeometry {
                viewport: Rect::EVERYTHING,
                canvas: Rect::from_min_size(Pos2::new(19.0, 23.0), Vec2::new(400.0, 300.0) * zoom),
                pixels_per_point: zoom,
            };
            let local = [73.0, 21.0];
            let screen = canvas.canvas_to_screen(path_local_to_canvas(&layer, geometry, local));
            let round_trip =
                canvas_to_path_local(&layer, geometry, canvas.screen_to_canvas(screen));
            assert!((round_trip.x - local[0]).abs() < 0.001);
            assert!((round_trip.y - local[1]).abs() < 0.001);
        }
    }

    #[test]
    fn reframing_past_every_edge_preserves_all_canvas_anchor_positions() {
        let layer = Layer {
            transform: Transform {
                x: 40.0,
                y: 25.0,
                scale_x: 1.4,
                scale_y: 0.8,
                rotation: 31.0,
            },
            kind: LayerKind::Path {
                geometry: path(),
                color: [255; 4],
            },
            ..Layer::default()
        };
        let LayerKind::Path { geometry, .. } = &layer.kind else {
            unreachable!()
        };
        for point in [[-25.0, 40.0], [135.0, 40.0], [50.0, -18.0], [50.0, 112.0]] {
            let before = path_local_to_canvas(&layer, geometry, geometry.anchors()[1].point);
            let (reframed, transform) = reframe_path_edit(
                &layer,
                geometry,
                0,
                PathAnchor {
                    point,
                    handle_in: [-9.0, -7.0],
                    handle_out: [11.0, 13.0],
                },
            )
            .unwrap();
            let updated = Layer {
                transform,
                kind: LayerKind::Path {
                    geometry: reframed.clone(),
                    color: [255; 4],
                },
                ..layer.clone()
            };
            let shifted_index = 1;
            let after =
                path_local_to_canvas(&updated, &reframed, reframed.anchors()[shifted_index].point);
            assert!(
                before.distance(after) < 0.001,
                "point {point:?}: {before:?} != {after:?}"
            );
        }
    }
}
