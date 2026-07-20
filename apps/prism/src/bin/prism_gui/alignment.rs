use super::*;

const SNAP_TOLERANCE_POINTS: f32 = 6.0;
const GUIDE_HIT_TOLERANCE_POINTS: f32 = 7.0;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct SmartGuides {
    pub vertical: Option<f32>,
    pub horizontal: Option<f32>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct SnappedMove {
    pub transform: Transform,
    pub guides: SmartGuides,
}

impl PrismApp {
    pub(super) fn snapped_move(
        &self,
        id: u64,
        transform: Transform,
        pixels_per_point: f32,
    ) -> SnappedMove {
        if !self.workspace.document.snapping_enabled {
            return SnappedMove {
                transform,
                guides: SmartGuides::default(),
            };
        }
        let Some(layer) = self.workspace.document.layer(id).ok() else {
            return SnappedMove {
                transform,
                guides: SmartGuides::default(),
            };
        };
        let Some(source_geometry) = self.layer_source_geometry(layer) else {
            return SnappedMove {
                transform,
                guides: SmartGuides::default(),
            };
        };
        let (target_x, target_y) = self.snap_targets(id);
        snap_layer_transform(
            layer,
            source_geometry,
            transform,
            &target_x,
            &target_y,
            pixels_per_point,
        )
    }

    fn snap_targets(&self, moving_id: u64) -> (Vec<f32>, Vec<f32>) {
        let document = &self.workspace.document;
        let mut x = document
            .guides
            .iter()
            .filter(|guide| guide.orientation == GuideOrientation::Vertical)
            .map(|guide| guide.position)
            .collect::<Vec<_>>();
        let mut y = document
            .guides
            .iter()
            .filter(|guide| guide.orientation == GuideOrientation::Horizontal)
            .map(|guide| guide.position)
            .collect::<Vec<_>>();
        x.extend([0.0, document.width as f32 * 0.5, document.width as f32]);
        y.extend([0.0, document.height as f32 * 0.5, document.height as f32]);
        for layer in document
            .layers
            .iter()
            .filter(|layer| layer.id != moving_id && layer.visible)
        {
            let Some(source) = self.layer_source_geometry(layer) else {
                continue;
            };
            let geometry = geometry_with_source(layer, source);
            x.extend([geometry.min[0], geometry.center[0], geometry.max[0]]);
            y.extend([geometry.min[1], geometry.center[1], geometry.max[1]]);
        }
        (x, y)
    }

    pub(super) fn guide_at_pointer(&self, geometry: CanvasGeometry, pointer: Pos2) -> Option<u64> {
        if !geometry
            .canvas
            .expand(GUIDE_HIT_TOLERANCE_POINTS)
            .contains(pointer)
        {
            return None;
        }
        self.workspace
            .document
            .guides
            .iter()
            .filter_map(|guide| {
                let distance = match guide.orientation {
                    GuideOrientation::Horizontal => (pointer.y
                        - geometry.canvas_to_screen(Pos2::new(0.0, guide.position)).y)
                        .abs(),
                    GuideOrientation::Vertical => (pointer.x
                        - geometry.canvas_to_screen(Pos2::new(guide.position, 0.0)).x)
                        .abs(),
                };
                (distance <= GUIDE_HIT_TOLERANCE_POINTS).then_some((guide.id, distance))
            })
            .min_by(|(_, left), (_, right)| left.total_cmp(right))
            .map(|(id, _)| id)
    }

    pub(super) fn guide_position(&self, id: u64, canvas: Pos2) -> Option<f32> {
        let guide = self.workspace.document.guide(id).ok()?;
        Some(match guide.orientation {
            GuideOrientation::Horizontal => canvas.y,
            GuideOrientation::Vertical => canvas.x,
        })
    }

    pub(super) fn guide_cursor(&self, id: u64) -> Option<egui::CursorIcon> {
        Some(match self.workspace.document.guide(id).ok()?.orientation {
            GuideOrientation::Horizontal => egui::CursorIcon::ResizeVertical,
            GuideOrientation::Vertical => egui::CursorIcon::ResizeHorizontal,
        })
    }

    pub(super) fn paint_alignment_guides(&self, ui: &egui::Ui, geometry: CanvasGeometry) {
        let active = self.drag.and_then(|drag| match drag.action {
            DragAction::Guide(id) => Some(id),
            _ => None,
        });
        let painter = ui
            .painter()
            .with_clip_rect(geometry.canvas.intersect(geometry.viewport));
        for guide in &self.workspace.document.guides {
            let position = guide.position;
            let points = match guide.orientation {
                GuideOrientation::Horizontal => [
                    geometry.canvas_to_screen(Pos2::new(0.0, position)),
                    geometry.canvas_to_screen(Pos2::new(
                        self.workspace.document.width as f32,
                        position,
                    )),
                ],
                GuideOrientation::Vertical => [
                    geometry.canvas_to_screen(Pos2::new(position, 0.0)),
                    geometry.canvas_to_screen(Pos2::new(
                        position,
                        self.workspace.document.height as f32,
                    )),
                ],
            };
            painter.line_segment(
                points,
                Stroke::new(
                    if active == Some(guide.id) { 1.5 } else { 1.0 },
                    with_alpha(ACCENT, if active == Some(guide.id) { 230 } else { 105 }),
                ),
            );
        }
        if let Some(x) = self.smart_guides.vertical {
            painter.line_segment(
                [
                    geometry.canvas_to_screen(Pos2::new(x, 0.0)),
                    geometry.canvas_to_screen(Pos2::new(x, self.workspace.document.height as f32)),
                ],
                Stroke::new(1.5, ACCENT),
            );
        }
        if let Some(y) = self.smart_guides.horizontal {
            painter.line_segment(
                [
                    geometry.canvas_to_screen(Pos2::new(0.0, y)),
                    geometry.canvas_to_screen(Pos2::new(self.workspace.document.width as f32, y)),
                ],
                Stroke::new(1.5, ACCENT),
            );
        }
    }

    pub(super) fn snapping_control(&mut self, ui: &mut egui::Ui) {
        let mut enabled = self.workspace.document.snapping_enabled;
        if ui.checkbox(&mut enabled, "Snap").changed() {
            self.execute(Command::SetSnapping { enabled });
        }
    }

    pub(super) fn alignment_inspector(&mut self, ui: &mut egui::Ui, layer: &Layer) {
        ui.add_space(8.0);
        ui.label(RichText::new("ALIGN").size(9.0).strong().color(SUBTLE));
        let references = self
            .workspace
            .document
            .layers
            .iter()
            .filter(|candidate| candidate.id != layer.id)
            .map(|candidate| (candidate.id, candidate.name.clone()))
            .collect::<Vec<_>>();
        if self
            .alignment_reference
            .is_some_and(|id| !references.iter().any(|(candidate, _)| *candidate == id))
        {
            self.alignment_reference = None;
        }
        egui::ComboBox::from_id_salt(("alignment-reference", layer.id))
            .selected_text(
                self.alignment_reference
                    .and_then(|id| {
                        references
                            .iter()
                            .find(|(candidate, _)| *candidate == id)
                            .map(|(_, name)| name.as_str())
                    })
                    .unwrap_or("Canvas"),
            )
            .width(ui.available_width())
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut self.alignment_reference, None, "Canvas");
                for (id, name) in &references {
                    ui.selectable_value(&mut self.alignment_reference, Some(*id), name);
                }
            });
        let mut chosen = None;
        ui.columns(3, |columns| {
            for (column, alignment, label) in [
                (0, Alignment::Left, "Left"),
                (1, Alignment::HorizontalCenter, "Center X"),
                (2, Alignment::Right, "Right"),
            ] {
                if columns[column]
                    .add_enabled(!layer.locked, egui::Button::new(label))
                    .clicked()
                {
                    chosen = Some(alignment);
                }
            }
        });
        ui.columns(3, |columns| {
            for (column, alignment, label) in [
                (0, Alignment::Top, "Top"),
                (1, Alignment::VerticalCenter, "Center Y"),
                (2, Alignment::Bottom, "Bottom"),
            ] {
                if columns[column]
                    .add_enabled(!layer.locked, egui::Button::new(label))
                    .clicked()
                {
                    chosen = Some(alignment);
                }
            }
        });
        if let Some(alignment) = chosen {
            self.execute(Command::AlignLayer {
                id: layer.id,
                alignment,
                reference: self
                    .alignment_reference
                    .map_or(AlignmentReference::Canvas, |id| AlignmentReference::Layer {
                        id,
                    }),
            });
        }
        self.guides_inspector(ui);
    }

    pub(super) fn guides_inspector(&mut self, ui: &mut egui::Ui) {
        ui.add_space(8.0);
        egui::CollapsingHeader::new("Guides")
            .id_salt("document-guides")
            .show(ui, |ui| {
                let mut snapping = self.workspace.document.snapping_enabled;
                if ui.checkbox(&mut snapping, "Snap while moving").changed() {
                    self.execute(Command::SetSnapping { enabled: snapping });
                }
                ui.horizontal(|ui| {
                    if ui.small_button("+ Vertical").clicked() {
                        self.execute(Command::AddGuide {
                            orientation: GuideOrientation::Vertical,
                            position: self.workspace.document.width as f32 * 0.5,
                        });
                    }
                    if ui.small_button("+ Horizontal").clicked() {
                        self.execute(Command::AddGuide {
                            orientation: GuideOrientation::Horizontal,
                            position: self.workspace.document.height as f32 * 0.5,
                        });
                    }
                });
                for guide in self.workspace.document.guides.clone() {
                    ui.horizontal(|ui| {
                        ui.label(match guide.orientation {
                            GuideOrientation::Horizontal => "H",
                            GuideOrientation::Vertical => "V",
                        });
                        let mut position = guide.position;
                        let response =
                            ui.add(egui::DragValue::new(&mut position).speed(1.0).suffix(" px"));
                        self.widget_command(
                            &response,
                            Command::MoveGuide {
                                id: guide.id,
                                position,
                            },
                        );
                        if ui.small_button("Remove").clicked() {
                            self.execute(Command::RemoveGuide { id: guide.id });
                        }
                    });
                }
                ui.label(
                    RichText::new("Drag guide lines directly on the canvas.")
                        .size(9.0)
                        .color(MUTED),
                );
            });
    }
}

#[derive(Clone, Copy)]
struct AxisSnap {
    delta: f32,
    target: f32,
}

fn closest_snap(moving: [f32; 3], targets: &[f32], tolerance: f32) -> Option<AxisSnap> {
    let mut best: Option<AxisSnap> = None;
    for target in targets {
        for anchor in moving {
            let candidate = AxisSnap {
                delta: *target - anchor,
                target: *target,
            };
            if candidate.delta.abs() <= tolerance
                && best.is_none_or(|best| candidate.delta.abs() < best.delta.abs())
            {
                best = Some(candidate);
            }
        }
    }
    best
}

fn snap_layer_transform(
    layer: &Layer,
    source_geometry: LayerSourceGeometry,
    transform: Transform,
    target_x: &[f32],
    target_y: &[f32],
    pixels_per_point: f32,
) -> SnappedMove {
    let mut moved = layer.clone();
    moved.transform = transform;
    let geometry = geometry_with_source(&moved, source_geometry);
    let tolerance = SNAP_TOLERANCE_POINTS / pixels_per_point.max(0.000_1);
    let snapped_x = closest_snap(
        [geometry.min[0], geometry.center[0], geometry.max[0]],
        target_x,
        tolerance,
    );
    let snapped_y = closest_snap(
        [geometry.min[1], geometry.center[1], geometry.max[1]],
        target_y,
        tolerance,
    );
    let mut transform = transform;
    if let Some(snap) = snapped_x {
        transform.x += snap.delta;
    }
    if let Some(snap) = snapped_y {
        transform.y += snap.delta;
    }
    SnappedMove {
        transform,
        guides: SmartGuides {
            vertical: snapped_x.map(|snap| snap.target),
            horizontal: snapped_y.map(|snap| snap.target),
        },
    }
}

fn geometry_with_source(layer: &Layer, source: LayerSourceGeometry) -> prism_core::LayerGeometry {
    prism_core::layer_geometry_with_bounds(
        layer,
        [source.visual_bounds.left(), source.visual_bounds.top()],
        [source.visual_bounds.width(), source.visual_bounds.height()],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snap_tolerance_stays_six_screen_points_at_every_zoom() {
        let moving = [97.0, 107.0, 117.0];
        let targets = [100.0];
        assert_eq!(closest_snap(moving, &targets, 6.0).unwrap().delta, 3.0);
        assert!(closest_snap(moving, &targets, 2.0).is_none());

        let low_zoom_canvas_delta = SNAP_TOLERANCE_POINTS / 0.25;
        let high_zoom_canvas_delta = SNAP_TOLERANCE_POINTS / 8.0;
        assert_eq!(low_zoom_canvas_delta * 0.25, SNAP_TOLERANCE_POINTS);
        assert_eq!(high_zoom_canvas_delta * 8.0, SNAP_TOLERANCE_POINTS);
    }

    #[test]
    fn exact_center_match_wins_over_nearby_edge_targets() {
        let snap = closest_snap([90.0, 100.0, 110.0], &[94.0, 100.0], 6.0).unwrap();
        assert_eq!(snap.delta, 0.0);
        assert_eq!(snap.target, 100.0);
    }

    #[test]
    fn rotated_bounds_snap_at_low_and_high_zoom_with_the_same_screen_tolerance() {
        let layer = Layer {
            transform: Transform {
                x: 200.0,
                y: 100.0,
                rotation: 37.0,
                ..Default::default()
            },
            kind: LayerKind::Rectangle {
                width: 120,
                height: 40,
                color: [255; 4],
                corner_radius: 0.0,
            },
            ..Default::default()
        };
        let source = LayerSourceGeometry::full(Vec2::new(120.0, 40.0));
        for pixels_per_point in [0.25, 8.0] {
            let geometry = geometry_with_source(&layer, source);
            let canvas_delta = 5.0 / pixels_per_point;
            let target = geometry.min[0] + canvas_delta;
            let snapped = snap_layer_transform(
                &layer,
                source,
                layer.transform,
                &[target],
                &[],
                pixels_per_point,
            );
            assert!((snapped.transform.x - layer.transform.x - canvas_delta).abs() < 0.001);
            assert_eq!(snapped.guides.vertical, Some(target));
            assert_eq!(snapped.guides.horizontal, None);
        }
    }

    #[test]
    fn rotated_bounds_do_not_snap_beyond_six_screen_points() {
        let layer = Layer {
            transform: Transform {
                x: 200.0,
                y: 100.0,
                rotation: 37.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let source = LayerSourceGeometry::full(Vec2::splat(100.0));
        let geometry = geometry_with_source(&layer, source);
        let target = geometry.min[0] + 6.1 / 8.0;
        let snapped = snap_layer_transform(&layer, source, layer.transform, &[target], &[], 8.0);
        assert_eq!(snapped.transform, layer.transform);
        assert_eq!(snapped.guides, SmartGuides::default());
    }

    #[test]
    fn visible_text_bounds_snap_at_low_and_high_zoom() {
        let layer = Layer {
            transform: Transform {
                x: 200.0,
                y: 100.0,
                rotation: 17.0,
                ..Default::default()
            },
            kind: LayerKind::Text {
                text: "Visible glyphs".into(),
                font_size: 48.0,
                color: [255; 4],
                typography: prism_core::TextTypography::default(),
            },
            ..Default::default()
        };
        let source = LayerSourceGeometry {
            size: Vec2::new(180.0, 72.0),
            visual_bounds: Rect::from_min_size(Pos2::new(14.0, 19.0), Vec2::new(142.0, 39.0)),
        };
        let full_source = LayerSourceGeometry::full(source.size);
        assert_ne!(
            geometry_with_source(&layer, source).min,
            geometry_with_source(&layer, full_source).min
        );

        for pixels_per_point in [0.25, 8.0] {
            let before = geometry_with_source(&layer, source);
            let canvas_delta = 5.0 / pixels_per_point;
            let target = before.min[0] + canvas_delta;
            let snapped = snap_layer_transform(
                &layer,
                source,
                layer.transform,
                &[target],
                &[],
                pixels_per_point,
            );
            let mut moved = layer.clone();
            moved.transform = snapped.transform;
            let after = geometry_with_source(&moved, source);

            assert!((after.min[0] - target).abs() < 0.001);
            assert_eq!(snapped.guides.vertical, Some(target));
        }
    }
}
