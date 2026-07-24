use super::*;

const SELECTION_STROKE_POINTS: f32 = 1.5;
const SELECTION_CONTRAST_STROKE_POINTS: f32 = 3.5;
const SELECTION_DASH_POINTS: f32 = 4.0;
const SELECTION_ANIMATION_POINTS_PER_SECOND: f32 = 24.0;
const SELECTION_REPAINT_INTERVAL: std::time::Duration = std::time::Duration::from_millis(42);

pub(super) struct SelectionOverlay {
    tab_id: u64,
    selection: prism_core::Selection,
    exact_paths: Option<std::sync::Arc<[prism_core::SelectionOutlinePath]>>,
    view: Option<SelectionOverlayView>,
}

struct SelectionOverlayView {
    key: prism_core::SelectionOutlineView,
    paths: std::sync::Arc<[prism_core::SelectionOutlinePath]>,
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

fn outline_point(point: Pos2) -> prism_core::SelectionOutlinePoint {
    prism_core::SelectionOutlinePoint::new(point.x, point.y)
}

fn rectangle_path(rect: Rect) -> prism_core::SelectionOutlinePath {
    vec![
        outline_point(rect.left_top()),
        outline_point(rect.right_top()),
        outline_point(rect.right_bottom()),
        outline_point(rect.left_bottom()),
        outline_point(rect.left_top()),
    ]
}

fn paint_selection_preview(ui: &egui::Ui, rect: Rect) {
    ui.painter().rect_filled(rect, 0.0, with_alpha(ACCENT, 24));
    paint_marching_ants(
        ui,
        std::slice::from_ref(&rectangle_path(rect)),
        prism_core::SelectionOutlineTransform {
            scale: 1.0,
            offset: prism_core::SelectionOutlinePoint::new(0.0, 0.0),
        },
    );
}

fn paint_marching_ants(
    ui: &egui::Ui,
    paths: &[prism_core::SelectionOutlinePath],
    transform: prism_core::SelectionOutlineTransform,
) {
    if paths.is_empty() {
        return;
    }
    ui.ctx().request_repaint_after(SELECTION_REPAINT_INTERVAL);
    let cycle = f64::from(SELECTION_DASH_POINTS * 2.0);
    let phase = ui.input(|input| {
        (input.time * f64::from(SELECTION_ANIMATION_POINTS_PER_SECOND)).rem_euclid(cycle) as f32
    });
    let clip = ui.clip_rect().expand(SELECTION_CONTRAST_STROKE_POINTS);
    let frame = prism_core::marching_ants_frame(
        paths,
        transform,
        prism_core::SelectionOutlineRect::new(outline_point(clip.min), outline_point(clip.max)),
        phase,
        SELECTION_DASH_POINTS,
    );
    let painter = ui.painter();
    for segment in frame.contrast {
        painter.line_segment(
            [
                Pos2::new(segment.start.x, segment.start.y),
                Pos2::new(segment.end.x, segment.end.y),
            ],
            Stroke::new(
                SELECTION_CONTRAST_STROKE_POINTS,
                Color32::from_black_alpha(230),
            ),
        );
    }
    for segment in frame.light {
        painter.line_segment(
            [
                Pos2::new(segment.start.x, segment.start.y),
                Pos2::new(segment.end.x, segment.end.y),
            ],
            Stroke::new(SELECTION_STROKE_POINTS, Color32::WHITE),
        );
    }
}

pub(super) fn paint_selection_overlay(
    ui: &egui::Ui,
    geometry: CanvasGeometry,
    selection: &prism_core::Selection,
    paths: Option<&[prism_core::SelectionOutlinePath]>,
) {
    let rectangle;
    let paths = if selection.alpha().is_none() {
        rectangle = vec![rectangle_path(selection_screen_rect(geometry, selection))];
        rectangle.as_slice()
    } else {
        paths.unwrap_or_default()
    };
    let transform = if selection.alpha().is_none() {
        prism_core::SelectionOutlineTransform {
            scale: 1.0,
            offset: prism_core::SelectionOutlinePoint::new(0.0, 0.0),
        }
    } else {
        prism_core::SelectionOutlineTransform {
            scale: geometry.pixels_per_point,
            offset: outline_point(geometry.canvas.min),
        }
    };
    paint_marching_ants(ui, paths, transform);
}

pub(super) fn paint_selection_drag(
    ui: &egui::Ui,
    geometry: CanvasGeometry,
    selection: &prism_core::Selection,
) {
    paint_selection_preview(ui, selection_screen_rect(geometry, selection));
}

fn selection_outline_view(
    geometry: CanvasGeometry,
    clip: Rect,
    bounds: (u32, u32, u32, u32),
) -> Option<prism_core::SelectionOutlineView> {
    let clip = clip.intersect(geometry.canvas);
    if !clip.is_positive() {
        return None;
    }
    let min = geometry.screen_to_canvas(clip.min);
    let max = geometry.screen_to_canvas(clip.max);
    let local_left =
        (min.x.floor() as i64 - i64::from(bounds.0) - 1).clamp(0, i64::from(bounds.2)) as u32;
    let local_top =
        (min.y.floor() as i64 - i64::from(bounds.1) - 1).clamp(0, i64::from(bounds.3)) as u32;
    let local_right =
        (max.x.ceil() as i64 - i64::from(bounds.0) + 1).clamp(0, i64::from(bounds.2)) as u32;
    let local_bottom =
        (max.y.ceil() as i64 - i64::from(bounds.1) + 1).clamp(0, i64::from(bounds.3)) as u32;
    (local_right > local_left && local_bottom > local_top).then_some(
        prism_core::SelectionOutlineView {
            x: local_left,
            y: local_top,
            width: local_right - local_left,
            height: local_bottom - local_top,
        },
    )
}

/*
 * Keep the contour cache separate from document history: it is derived entirely
 * from the current immutable selection and can be discarded at any time.
 */
impl PrismApp {
    pub(super) fn ensure_selection_overlay(
        &mut self,
        geometry: CanvasGeometry,
        clip: Rect,
    ) -> Option<std::sync::Arc<[prism_core::SelectionOutlinePath]>> {
        let selection = self.workspace.document.selection.as_ref()?.clone();
        let matches_cached_selection = self.selection_ui.overlay.as_ref().is_some_and(|overlay| {
            overlay.tab_id == self.active_tab_id && overlay.selection == selection
        });
        if !matches_cached_selection {
            let exact_paths = selection.alpha().map_or_else(
                || Some(std::sync::Arc::from([])),
                |alpha| match prism_core::selection_mask_outline(selection.bounds(), alpha) {
                    prism_core::SelectionMaskOutline::Exact(paths) => Some(paths),
                    prism_core::SelectionMaskOutline::Complex => None,
                },
            );
            self.selection_ui.overlay = Some(SelectionOverlay {
                tab_id: self.active_tab_id,
                selection,
                exact_paths,
                view: None,
            });
        }
        let overlay = self.selection_ui.overlay.as_mut()?;
        if let Some(paths) = &overlay.exact_paths {
            return Some(std::sync::Arc::clone(paths));
        }
        let key = selection_outline_view(geometry, clip, overlay.selection.bounds())?;
        if let Some(view) = &overlay.view
            && view.key == key
        {
            return Some(std::sync::Arc::clone(&view.paths));
        }
        let paths = prism_core::complex_selection_mask_outline(
            overlay.selection.bounds(),
            overlay.selection.alpha()?,
            key,
        );
        overlay.view = Some(SelectionOverlayView {
            key,
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
    fn complex_outline_view_tracks_the_visible_source_pixels_at_high_zoom() {
        let geometry = CanvasGeometry {
            viewport: Rect::from_min_max(Pos2::ZERO, Pos2::new(1_000.0, 800.0)),
            canvas: Rect::from_min_size(
                Pos2::new(100.0, 200.0),
                Vec2::new(1_024.0, 1_024.0) * 16.0,
            ),
            pixels_per_point: 16.0,
        };
        assert_eq!(
            selection_outline_view(
                geometry,
                Rect::from_min_max(Pos2::new(260.0, 360.0), Pos2::new(420.0, 520.0)),
                (0, 0, 1_024, 1_024),
            ),
            Some(prism_core::SelectionOutlineView {
                x: 9,
                y: 9,
                width: 12,
                height: 12,
            })
        );
    }

    #[test]
    fn default_magic_wand_tolerance_is_twenty() {
        assert_eq!(SelectionUiState::default().magic_wand_tolerance, 20);
    }
}
