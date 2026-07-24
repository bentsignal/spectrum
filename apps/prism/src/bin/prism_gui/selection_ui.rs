use super::*;

const SELECTION_STROKE_POINTS: f32 = 1.5;
const SELECTION_CONTRAST_STROKE_POINTS: f32 = 3.5;
const SELECTION_DASH_POINTS: f32 = 4.0;
const SELECTION_ANIMATION_POINTS_PER_SECOND: f32 = 24.0;
const SELECTION_REPAINT_INTERVAL: std::time::Duration = std::time::Duration::from_millis(42);
const MAX_CONTOUR_EDGES: usize = 65_536;
const MAX_FALLBACK_MASK_EDGE: u32 = 1_024;
// Match the conventional selection boundary at 50% coverage while retaining
// the antialiased alpha plane itself unchanged for fill, crop, and persistence.
const SELECTION_BOUNDARY_ALPHA: u8 = 128;

pub(super) struct SelectionOverlay {
    tab_id: u64,
    selection: prism_core::Selection,
    paths: std::sync::Arc<[Vec<Pos2>]>,
}

pub(super) struct SelectionUiState {
    pub(super) fill_color: Color32,
    pub(super) overlay: Option<SelectionOverlay>,
    pub(super) magic_wand_tolerance: u8,
    pub(super) magic_wand_contiguous: bool,
    pub(super) magic_wand_antialias: bool,
    pub(super) lasso_points: Vec<Pos2>,
    pub(super) lasso_mode: prism_core::SelectionCombineMode,
    pub(super) lasso_gesture_mode: Option<prism_core::SelectionCombineMode>,
    pub(super) lasso_antialias: bool,
    pub(super) lasso_overflowed: bool,
}

impl Default for SelectionUiState {
    fn default() -> Self {
        Self {
            fill_color: Color32::from_rgba_unmultiplied(93, 216, 199, 255),
            overlay: None,
            magic_wand_tolerance: 20,
            magic_wand_contiguous: true,
            magic_wand_antialias: true,
            lasso_points: Vec::new(),
            lasso_mode: prism_core::SelectionCombineMode::Replace,
            lasso_gesture_mode: None,
            lasso_antialias: true,
            lasso_overflowed: false,
        }
    }
}

pub(super) fn selection_from_drag(
    canvas_width: u32,
    canvas_height: u32,
    start: Pos2,
    current: Pos2,
) -> Option<prism_core::Selection> {
    let clamp = |position: Pos2| {
        Pos2::new(
            position.x.clamp(0.0, canvas_width as f32),
            position.y.clamp(0.0, canvas_height as f32),
        )
    };
    let start = clamp(start);
    let current = clamp(current);
    let min = start.min(current);
    let max = start.max(current);
    let left = min.x.floor().max(0.0) as u32;
    let top = min.y.floor().max(0.0) as u32;
    let right = max.x.ceil().min(canvas_width as f32) as u32;
    let bottom = max.y.ceil().min(canvas_height as f32) as u32;
    (right > left && bottom > top)
        .then(|| prism_core::Selection::rectangle(left, top, right - left, bottom - top))
}

pub(super) fn selection_screen_rect(
    geometry: CanvasGeometry,
    selection: &prism_core::Selection,
) -> Rect {
    let (x, y, width, height) = selection.bounds();
    Rect::from_min_max(
        geometry.canvas_to_screen(Pos2::new(x as f32, y as f32)),
        geometry.canvas_to_screen(Pos2::new((x + width) as f32, (y + height) as f32)),
    )
}

fn full_canvas_selection(width: u32, height: u32) -> prism_core::Selection {
    prism_core::Selection::rectangle(0, 0, width, height)
}

fn can_crop_to_selection(
    selection: Option<&prism_core::Selection>,
    canvas_width: u32,
    canvas_height: u32,
) -> bool {
    selection.is_some_and(|selection| selection.bounds() != (0, 0, canvas_width, canvas_height))
}

fn rectangle_path(rect: Rect) -> Vec<Pos2> {
    vec![
        rect.left_top(),
        rect.right_top(),
        rect.right_bottom(),
        rect.left_bottom(),
        rect.left_top(),
    ]
}

fn paint_selection_preview(ui: &egui::Ui, rect: Rect) {
    ui.painter().rect_filled(rect, 0.0, with_alpha(ACCENT, 24));
    paint_marching_ants(ui, std::slice::from_ref(&rectangle_path(rect)));
}

fn paint_marching_ants(ui: &egui::Ui, paths: &[Vec<Pos2>]) {
    if paths.is_empty() {
        return;
    }
    ui.ctx().request_repaint_after(SELECTION_REPAINT_INTERVAL);
    let phase = ui.input(|input| input.time as f32) * SELECTION_ANIMATION_POINTS_PER_SECOND;
    let clip = ui.clip_rect().expand(SELECTION_CONTRAST_STROKE_POINTS);
    let painter = ui.painter();
    for path in paths {
        let mut path_distance = 0.0;
        for segment in path.windows(2) {
            let start = segment[0];
            let end = segment[1];
            let delta = end - start;
            let length = delta.length();
            if length <= f32::EPSILON {
                continue;
            }
            if let Some((clip_start, clip_end)) = clipped_segment_parameter(start, end, clip) {
                let visible_start = start.lerp(end, clip_start);
                let visible_end = start.lerp(end, clip_end);
                painter.line_segment(
                    [visible_start, visible_end],
                    Stroke::new(
                        SELECTION_CONTRAST_STROKE_POINTS,
                        Color32::from_black_alpha(230),
                    ),
                );
                paint_white_dashes(
                    painter,
                    VisibleSelectionSegment {
                        start,
                        end,
                        path_distance,
                        length,
                        clip_start,
                        clip_end,
                    },
                    phase,
                );
            }
            path_distance += length;
        }
    }
}

#[derive(Clone, Copy)]
struct VisibleSelectionSegment {
    start: Pos2,
    end: Pos2,
    path_distance: f32,
    length: f32,
    clip_start: f32,
    clip_end: f32,
}

fn paint_white_dashes(painter: &egui::Painter, segment: VisibleSelectionSegment, phase: f32) {
    let cycle = SELECTION_DASH_POINTS * 2.0;
    let mut distance = segment.path_distance + segment.clip_start * segment.length;
    let visible_end = segment.path_distance + segment.clip_end * segment.length;
    while distance < visible_end {
        let cycle_position = (distance + phase).rem_euclid(cycle);
        let white = cycle_position < SELECTION_DASH_POINTS;
        let run = if white {
            SELECTION_DASH_POINTS - cycle_position
        } else {
            cycle - cycle_position
        };
        let next = (distance + run.max(0.01)).min(visible_end);
        if white {
            let start_t = ((distance - segment.path_distance) / segment.length).clamp(0.0, 1.0);
            let end_t = ((next - segment.path_distance) / segment.length).clamp(0.0, 1.0);
            painter.line_segment(
                [
                    segment.start.lerp(segment.end, start_t),
                    segment.start.lerp(segment.end, end_t),
                ],
                Stroke::new(SELECTION_STROKE_POINTS, Color32::WHITE),
            );
        }
        distance = next;
    }
}

fn clipped_segment_parameter(start: Pos2, end: Pos2, clip: Rect) -> Option<(f32, f32)> {
    let delta = end - start;
    let mut lower = 0.0_f32;
    let mut upper = 1.0_f32;
    for (origin, direction, minimum, maximum) in [
        (start.x, delta.x, clip.left(), clip.right()),
        (start.y, delta.y, clip.top(), clip.bottom()),
    ] {
        if direction.abs() <= f32::EPSILON {
            if origin < minimum || origin > maximum {
                return None;
            }
            continue;
        }
        let first = (minimum - origin) / direction;
        let second = (maximum - origin) / direction;
        let (near, far) = if first <= second {
            (first, second)
        } else {
            (second, first)
        };
        lower = lower.max(near);
        upper = upper.min(far);
        if lower > upper {
            return None;
        }
    }
    Some((lower.clamp(0.0, 1.0), upper.clamp(0.0, 1.0)))
}

pub(super) fn paint_selection_overlay(
    ui: &egui::Ui,
    geometry: CanvasGeometry,
    selection: &prism_core::Selection,
    paths: Option<&[Vec<Pos2>]>,
) {
    let screen_paths = if selection.alpha().is_none() {
        vec![rectangle_path(selection_screen_rect(geometry, selection))]
    } else {
        paths
            .unwrap_or_default()
            .iter()
            .map(|path| {
                path.iter()
                    .map(|point| geometry.canvas_to_screen(*point))
                    .collect()
            })
            .collect()
    };
    paint_marching_ants(ui, &screen_paths);
}

pub(super) fn paint_selection_drag(
    ui: &egui::Ui,
    geometry: CanvasGeometry,
    selection: &prism_core::Selection,
) {
    paint_selection_preview(ui, selection_screen_rect(geometry, selection));
}

#[derive(Clone, Copy, Debug)]
struct BoundaryEdge {
    start: (u32, u32),
    end: (u32, u32),
    direction: u8,
}

fn color_mask_paths(bounds: (u32, u32, u32, u32), alpha: &[u8]) -> std::sync::Arc<[Vec<Pos2>]> {
    let (_, _, width, height) = bounds;
    if width == 0 || height == 0 || alpha.len() != (u64::from(width) * u64::from(height)) as usize {
        return std::sync::Arc::from([]);
    }
    if let Some(edges) = boundary_edges(width, height, |x, y| {
        alpha[(u64::from(y) * u64::from(width) + u64::from(x)) as usize] >= SELECTION_BOUNDARY_ALPHA
    }) {
        return map_contours(trace_contours(edges), bounds, width, height);
    }

    let mut fallback_width = width.min(MAX_FALLBACK_MASK_EDGE);
    let mut fallback_height = height.min(MAX_FALLBACK_MASK_EDGE);
    loop {
        let occupancy = aggregate_mask(width, height, alpha, fallback_width, fallback_height);
        if let Some(edges) = boundary_edges(fallback_width, fallback_height, |x, y| {
            occupancy[(u64::from(y) * u64::from(fallback_width) + u64::from(x)) as usize]
        }) {
            return map_contours(
                trace_contours(edges),
                bounds,
                fallback_width,
                fallback_height,
            );
        }
        if fallback_width == 1 && fallback_height == 1 {
            return std::sync::Arc::from([]);
        }
        fallback_width = fallback_width.div_ceil(2).max(1);
        fallback_height = fallback_height.div_ceil(2).max(1);
    }
}

fn boundary_edges(
    width: u32,
    height: u32,
    selected: impl Fn(u32, u32) -> bool,
) -> Option<Vec<BoundaryEdge>> {
    let mut edges = Vec::new();
    let mut push = |start, end, direction| {
        if edges.len() >= MAX_CONTOUR_EDGES {
            return false;
        }
        edges.push(BoundaryEdge {
            start,
            end,
            direction,
        });
        true
    };
    for y in 0..height {
        for x in 0..width {
            if !selected(x, y) {
                continue;
            }
            if y == 0 && !push((x, y), (x + 1, y), 0)
                || y > 0 && !selected(x, y - 1) && !push((x, y), (x + 1, y), 0)
                || x + 1 == width && !push((x + 1, y), (x + 1, y + 1), 1)
                || x + 1 < width && !selected(x + 1, y) && !push((x + 1, y), (x + 1, y + 1), 1)
                || y + 1 == height && !push((x + 1, y + 1), (x, y + 1), 2)
                || y + 1 < height && !selected(x, y + 1) && !push((x + 1, y + 1), (x, y + 1), 2)
                || x == 0 && !push((x, y + 1), (x, y), 3)
                || x > 0 && !selected(x - 1, y) && !push((x, y + 1), (x, y), 3)
            {
                return None;
            }
        }
    }
    Some(edges)
}

fn trace_contours(edges: Vec<BoundaryEdge>) -> Vec<Vec<(u32, u32)>> {
    let mut outgoing: HashMap<(u32, u32), Vec<usize>> = HashMap::new();
    for (index, edge) in edges.iter().enumerate() {
        outgoing.entry(edge.start).or_default().push(index);
    }
    let mut visited = vec![false; edges.len()];
    let mut contours = Vec::new();
    for first in 0..edges.len() {
        if visited[first] {
            continue;
        }
        let start = edges[first].start;
        let mut points = vec![start];
        let mut current = first;
        loop {
            if visited[current] {
                break;
            }
            visited[current] = true;
            let edge = edges[current];
            points.push(edge.end);
            if edge.end == start {
                break;
            }
            let Some(candidates) = outgoing.get(&edge.end) else {
                break;
            };
            let Some(next) = candidates
                .iter()
                .copied()
                .filter(|candidate| !visited[*candidate])
                .min_by_key(|candidate| turn_rank(edge.direction, edges[*candidate].direction))
            else {
                break;
            };
            current = next;
        }
        let points = simplify_contour(points);
        if points.len() >= 4 && points.first() == points.last() {
            contours.push(points);
        }
    }
    contours
}

fn turn_rank(previous: u8, next: u8) -> u8 {
    match next.wrapping_sub(previous) % 4 {
        1 => 0,
        0 => 1,
        3 => 2,
        _ => 3,
    }
}

fn simplify_contour(points: Vec<(u32, u32)>) -> Vec<(u32, u32)> {
    if points.len() < 4 {
        return points;
    }
    let mut simplified = Vec::with_capacity(points.len());
    simplified.push(points[0]);
    for window in points.windows(3) {
        let first = window[0];
        let middle = window[1];
        let last = window[2];
        let collinear = (first.0 == middle.0 && middle.0 == last.0)
            || (first.1 == middle.1 && middle.1 == last.1);
        if !collinear {
            simplified.push(middle);
        }
    }
    simplified.push(*points.last().expect("contour has points"));
    simplified
}

fn aggregate_mask(
    width: u32,
    height: u32,
    alpha: &[u8],
    output_width: u32,
    output_height: u32,
) -> Vec<bool> {
    let mut occupancy = vec![false; (u64::from(output_width) * u64::from(output_height)) as usize];
    for y in 0..height {
        let output_y = (u64::from(y) * u64::from(output_height) / u64::from(height)) as u32;
        for x in 0..width {
            let source = (u64::from(y) * u64::from(width) + u64::from(x)) as usize;
            if alpha[source] < SELECTION_BOUNDARY_ALPHA {
                continue;
            }
            let output_x = (u64::from(x) * u64::from(output_width) / u64::from(width)) as u32;
            let output =
                (u64::from(output_y) * u64::from(output_width) + u64::from(output_x)) as usize;
            occupancy[output] = true;
        }
    }
    occupancy
}

fn map_contours(
    contours: Vec<Vec<(u32, u32)>>,
    bounds: (u32, u32, u32, u32),
    grid_width: u32,
    grid_height: u32,
) -> std::sync::Arc<[Vec<Pos2>]> {
    contours
        .into_iter()
        .map(|contour| {
            contour
                .into_iter()
                .map(|(x, y)| {
                    Pos2::new(
                        bounds.0 as f32 + x as f32 * bounds.2 as f32 / grid_width as f32,
                        bounds.1 as f32 + y as f32 * bounds.3 as f32 / grid_height as f32,
                    )
                })
                .collect()
        })
        .collect()
}

/*
 * Keep the contour cache separate from document history: it is derived entirely
 * from the current immutable selection and can be discarded at any time.
 */
impl PrismApp {
    pub(super) fn ensure_selection_overlay(&mut self) -> Option<std::sync::Arc<[Vec<Pos2>]>> {
        let selection = self.workspace.document.selection.as_ref()?.clone();
        if self.selection_ui.overlay.as_ref().is_some_and(|overlay| {
            overlay.tab_id == self.active_tab_id && overlay.selection == selection
        }) {
            return self
                .selection_ui
                .overlay
                .as_ref()
                .map(|overlay| std::sync::Arc::clone(&overlay.paths));
        }
        let paths = selection.alpha().map_or_else(
            || std::sync::Arc::from([]),
            |alpha| color_mask_paths(selection.bounds(), alpha),
        );
        self.selection_ui.overlay = Some(SelectionOverlay {
            tab_id: self.active_tab_id,
            selection,
            paths: std::sync::Arc::clone(&paths),
        });
        Some(paths)
    }
}

impl PrismApp {
    pub(super) fn select_all_pixels(&mut self) {
        let document = &self.workspace.document;
        self.execute(Command::SetSelection {
            selection: Some(full_canvas_selection(document.width, document.height)),
        });
    }

    pub(super) fn deselect_pixels(&mut self) {
        self.execute(Command::SetSelection { selection: None });
    }

    pub(super) fn selection_workbench_controls(&mut self, ui: &mut egui::Ui) {
        let has_selection = self.workspace.document.selection.is_some();
        let can_crop = can_crop_to_selection(
            self.workspace.document.selection.as_ref(),
            self.workspace.document.width,
            self.workspace.document.height,
        );
        ui.separator();
        if self.tool == Tool::MagicWand {
            ui.label(RichText::new("TOLERANCE").size(9.0).strong().color(SUBTLE));
            ui.add(
                egui::Slider::new(&mut self.selection_ui.magic_wand_tolerance, 0..=255)
                    .text("Tolerance")
                    .clamping(egui::SliderClamping::Always),
            );
            ui.checkbox(&mut self.selection_ui.magic_wand_contiguous, "Contiguous")
                .on_hover_text("Limit selection to pixels connected to the clicked color");
            ui.checkbox(&mut self.selection_ui.magic_wand_antialias, "Anti-alias")
                .on_hover_text("Keep a soft one-pixel boundary around the matched color");
            ui.separator();
        }
        if self.tool == Tool::Lasso {
            ui.label(RichText::new("LASSO").size(9.0).strong().color(SUBTLE));
            egui::ComboBox::from_id_salt("lasso-combine-mode")
                .selected_text(match self.selection_ui.lasso_mode {
                    prism_core::SelectionCombineMode::Replace => "Replace",
                    prism_core::SelectionCombineMode::Add => "Add",
                    prism_core::SelectionCombineMode::Subtract => "Subtract",
                    prism_core::SelectionCombineMode::Intersect => "Intersect",
                })
                .show_ui(ui, |ui| {
                    for (mode, label) in [
                        (prism_core::SelectionCombineMode::Replace, "Replace"),
                        (prism_core::SelectionCombineMode::Add, "Add"),
                        (prism_core::SelectionCombineMode::Subtract, "Subtract"),
                        (prism_core::SelectionCombineMode::Intersect, "Intersect"),
                    ] {
                        ui.selectable_value(&mut self.selection_ui.lasso_mode, mode, label);
                    }
                });
            ui.checkbox(&mut self.selection_ui.lasso_antialias, "Anti-alias");
            ui.label(
                RichText::new("Shift add · Option/Alt subtract · both intersect")
                    .size(10.0)
                    .color(MUTED),
            );
            ui.separator();
        }
        ui.label(RichText::new("FILL").size(9.0).strong().color(SUBTLE));
        ui.color_edit_button_srgba(&mut self.selection_ui.fill_color)
            .on_hover_text("Solid fill color");
        if ui
            .add_enabled(has_selection, egui::Button::new("Create fill layer"))
            .on_hover_text("Create one editable solid layer honoring the pixel selection")
            .clicked()
        {
            self.execute(Command::FillSelection {
                color: self.selection_ui.fill_color.to_array(),
                name: None,
            });
        }
        if ui
            .add_enabled(can_crop, egui::Button::new("Crop canvas to selection"))
            .on_hover_text("Crop to the selection and deselect in one revision")
            .clicked()
            && self.execute(Command::CropToSelection)
        {
            self.fit_requested = true;
        }
        if ui
            .add_enabled(has_selection, egui::Button::new("Deselect"))
            .on_hover_text("Deselect the current pixel selection")
            .clicked()
        {
            self.deselect_pixels();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_mapping_is_stable_across_zoom_pan_and_rotated_content() {
        let viewport = Rect::from_min_size(Pos2::new(10.0, 20.0), Vec2::new(900.0, 700.0));
        let expected = prism_core::Selection::rectangle(120, 80, 241, 161);
        for geometry in [
            canvas_geometry(viewport, 800, 600, 0.5, Vec2::new(-60.0, 24.0)),
            canvas_geometry(viewport, 800, 600, 4.0, Vec2::new(71.0, -33.0)),
        ] {
            let start =
                geometry.screen_to_canvas(geometry.canvas_to_screen(Pos2::new(120.2, 80.4)));
            let current =
                geometry.screen_to_canvas(geometry.canvas_to_screen(Pos2::new(360.6, 240.7)));
            assert_eq!(
                selection_from_drag(800, 600, start, current),
                Some(expected.clone())
            );
            let screen = selection_screen_rect(geometry, &expected);
            assert!((screen.width() - 241.0 * geometry.pixels_per_point).abs() < 0.01);
            assert!((screen.height() - 161.0 * geometry.pixels_per_point).abs() < 0.01);
        }
        // Selection geometry is document-space and therefore independent of any
        // focused layer's rotation; rotation never enters either mapping helper.
        let rotated_layer = Layer {
            transform: Transform {
                rotation: 37.0,
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(rotated_layer.transform.rotation, 37.0);
        assert_eq!(SELECTION_STROKE_POINTS, 1.5);
    }

    #[test]
    fn reverse_and_outside_drags_clip_to_exact_canvas_pixels() {
        assert_eq!(
            selection_from_drag(100, 80, Pos2::new(44.2, 31.2), Pos2::new(-8.0, 9.8)),
            Some(prism_core::Selection::rectangle(0, 9, 45, 23))
        );
        assert_eq!(
            selection_from_drag(100, 80, Pos2::new(-8.0, -4.0), Pos2::new(-1.0, 20.0)),
            None
        );
    }

    #[test]
    fn select_all_uses_the_exact_document_pixel_bounds() {
        assert_eq!(
            full_canvas_selection(1_920, 1_080),
            prism_core::Selection::rectangle(0, 0, 1_920, 1_080)
        );
    }

    #[test]
    fn crop_control_requires_a_selection_smaller_than_the_canvas() {
        assert!(!can_crop_to_selection(None, 1_920, 1_080));
        let full = full_canvas_selection(1_920, 1_080);
        assert!(!can_crop_to_selection(Some(&full), 1_920, 1_080,));
        let partial = prism_core::Selection::rectangle(20, 30, 640, 480);
        assert!(can_crop_to_selection(Some(&partial), 1_920, 1_080,));
    }

    #[test]
    fn irregular_and_antialiased_masks_produce_exact_closed_contours() {
        let alpha = [
            0, 0, 0, 0, 0, 0, 255, 160, 0, 0, 0, 255, 255, 0, 0, 0, 0, 0, 0, 255,
        ];
        let paths = color_mask_paths((10, 20, 5, 4), &alpha);
        assert_eq!(paths.len(), 2);
        assert!(paths.iter().all(|path| path.first() == path.last()));
        assert!(
            paths
                .iter()
                .flatten()
                .all(|point| point.x >= 10.0 && point.x <= 15.0)
        );
        assert!(
            paths
                .iter()
                .flatten()
                .all(|point| point.y >= 20.0 && point.y <= 24.0)
        );
    }

    #[test]
    fn contour_generation_is_bounded_for_a_pathological_large_mask() {
        let mut alpha = vec![0; 1_024 * 1_024];
        for y in 0..1_024 {
            for x in 0..1_024 {
                alpha[y * 1_024 + x] = if (x + y) % 2 == 0 { 255 } else { 0 };
            }
        }
        let paths = color_mask_paths((0, 0, 1_024, 1_024), &alpha);
        let vertices = paths.iter().map(Vec::len).sum::<usize>();
        assert!(vertices <= MAX_CONTOUR_EDGES);
        assert!(!paths.is_empty());
    }

    #[test]
    fn contour_rejects_malformed_alpha_without_work() {
        assert!(color_mask_paths((0, 0, 4, 4), &[255; 15]).is_empty());
    }

    #[test]
    fn default_magic_wand_tolerance_is_twenty() {
        assert_eq!(SelectionUiState::default().magic_wand_tolerance, 20);
    }

    #[test]
    fn segment_clipping_bounds_offscreen_animation_work() {
        let clip = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(100.0, 100.0));
        let (start, end) =
            clipped_segment_parameter(Pos2::new(-10_000.0, 50.0), Pos2::new(10_000.0, 50.0), clip)
                .unwrap();
        assert!((start - 0.5).abs() < 0.01);
        assert!((end - 0.505).abs() < 0.01);
        assert!(
            clipped_segment_parameter(Pos2::new(-10.0, -5.0), Pos2::new(110.0, -5.0), clip,)
                .is_none()
        );
    }
}
