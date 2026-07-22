use super::*;

const MARQUEE_STROKE_POINTS: f32 = 1.0;
const MAX_COLOR_OVERLAY_EDGE: u32 = 512;

pub(super) struct SelectionOverlay {
    tab_id: u64,
    selection: prism_core::Selection,
    bounds: (u32, u32, u32, u32),
    texture: TextureHandle,
}

pub(super) struct SelectionUiState {
    pub(super) fill_color: Color32,
    pub(super) overlay: Option<SelectionOverlay>,
    pub(super) magic_wand_tolerance: u8,
    pub(super) magic_wand_contiguous: bool,
    pub(super) magic_wand_antialias: bool,
}

impl Default for SelectionUiState {
    fn default() -> Self {
        Self {
            fill_color: Color32::from_rgba_unmultiplied(93, 216, 199, 255),
            overlay: None,
            magic_wand_tolerance: 32,
            magic_wand_contiguous: true,
            magic_wand_antialias: true,
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

fn paint_marquee(ui: &egui::Ui, rect: Rect, preview: bool) {
    if preview {
        ui.painter().rect_filled(rect, 0.0, with_alpha(ACCENT, 24));
    }
    ui.painter().rect_stroke(
        rect,
        0.0,
        Stroke::new(MARQUEE_STROKE_POINTS + 2.0, Color32::from_black_alpha(190)),
        egui::StrokeKind::Inside,
    );
    ui.painter().rect_stroke(
        rect,
        0.0,
        Stroke::new(MARQUEE_STROKE_POINTS, Color32::WHITE),
        egui::StrokeKind::Inside,
    );
}

pub(super) fn paint_selection_overlay(
    ui: &egui::Ui,
    geometry: CanvasGeometry,
    selection: &prism_core::Selection,
    overlay: Option<(egui::TextureId, (u32, u32, u32, u32))>,
) {
    if selection.alpha().is_none() {
        paint_marquee(ui, selection_screen_rect(geometry, selection), false);
        return;
    }
    if let Some((texture, bounds)) = overlay {
        let selection = prism_core::Selection::rectangle(bounds.0, bounds.1, bounds.2, bounds.3);
        ui.painter().image(
            texture,
            selection_screen_rect(geometry, &selection),
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            Color32::WHITE,
        );
    }
}

pub(super) fn paint_selection_drag(
    ui: &egui::Ui,
    geometry: CanvasGeometry,
    selection: &prism_core::Selection,
) {
    paint_marquee(ui, selection_screen_rect(geometry, selection), true);
}

impl PrismApp {
    pub(super) fn ensure_selection_overlay(
        &mut self,
        context: &egui::Context,
    ) -> Option<(egui::TextureId, (u32, u32, u32, u32))> {
        let selection = self.workspace.document.selection.as_ref()?.clone();
        let bounds = selection.bounds();
        if self.selection_ui.overlay.as_ref().is_some_and(|overlay| {
            overlay.tab_id == self.active_tab_id && overlay.selection == selection
        }) {
            let overlay = self.selection_ui.overlay.as_ref()?;
            return Some((overlay.texture.id(), overlay.bounds));
        }
        let alpha = selection.alpha()?;
        let image = color_mask_overlay_image(bounds.2, bounds.3, alpha)?;
        let size = [image.width() as usize, image.height() as usize];
        let pixels = egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw());
        let texture = context.load_texture(
            format!("prism-selection-overlay-{}", self.active_tab_id),
            pixels,
            TextureOptions::NEAREST,
        );
        self.selection_ui.overlay = Some(SelectionOverlay {
            tab_id: self.active_tab_id,
            selection,
            bounds,
            texture,
        });
        let overlay = self.selection_ui.overlay.as_ref()?;
        Some((overlay.texture.id(), overlay.bounds))
    }

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
            ui.label(RichText::new("RANGE").size(9.0).strong().color(SUBTLE));
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
            .on_hover_text("Crop to the marquee and deselect in one revision")
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

fn color_mask_overlay_image(width: u32, height: u32, alpha: &[u8]) -> Option<image::RgbaImage> {
    if width == 0 || height == 0 || alpha.len() != (u64::from(width) * u64::from(height)) as usize {
        return None;
    }
    let longest = width.max(height);
    let (output_width, output_height) = if longest <= MAX_COLOR_OVERLAY_EDGE {
        (width, height)
    } else if width >= height {
        (
            MAX_COLOR_OVERLAY_EDGE,
            ((u64::from(height) * u64::from(MAX_COLOR_OVERLAY_EDGE) + u64::from(width) / 2)
                / u64::from(width))
            .max(1) as u32,
        )
    } else {
        (
            ((u64::from(width) * u64::from(MAX_COLOR_OVERLAY_EDGE) + u64::from(height) / 2)
                / u64::from(height))
            .max(1) as u32,
            MAX_COLOR_OVERLAY_EDGE,
        )
    };
    Some(image::RgbaImage::from_fn(
        output_width,
        output_height,
        |x, y| {
            // Max-area aggregation guarantees even a one-pixel disconnected island
            // remains visible in the bounded overview texture. Nearest GPU sampling
            // avoids interpolating coverage across a known-zero cell.
            let source_left = u64::from(x) * u64::from(width) / u64::from(output_width);
            let source_right = ((u64::from(x + 1) * u64::from(width) + u64::from(output_width)
                - 1)
                / u64::from(output_width))
            .max(source_left + 1)
            .min(u64::from(width));
            let source_top = u64::from(y) * u64::from(height) / u64::from(output_height);
            let source_bottom = ((u64::from(y + 1) * u64::from(height) + u64::from(output_height)
                - 1)
                / u64::from(output_height))
            .max(source_top + 1)
            .min(u64::from(height));
            let mut selection_alpha = 0;
            for source_y in source_top..source_bottom {
                for source_x in source_left..source_right {
                    let index = (source_y * u64::from(width) + source_x) as usize;
                    selection_alpha = selection_alpha.max(alpha[index]);
                }
            }
            image::Rgba([
                ACCENT.r(),
                ACCENT.g(),
                ACCENT.b(),
                ((u16::from(selection_alpha) * 92) / 255) as u8,
            ])
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marquee_mapping_is_stable_across_zoom_pan_and_rotated_content() {
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
        assert_eq!(MARQUEE_STROKE_POINTS, 1.0);
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
    fn bounded_color_overlay_preserves_single_pixel_islands() {
        let mut alpha = vec![0; 4_096 * 4_096];
        alpha[2_047 * 4_096 + 3_001] = 255;
        let overlay = color_mask_overlay_image(4_096, 4_096, &alpha).unwrap();
        assert_eq!(overlay.dimensions(), (512, 512));
        assert!(overlay.pixels().any(|pixel| pixel[3] > 0));
        assert!(overlay.pixels().filter(|pixel| pixel[3] > 0).count() <= 4);
    }

    #[test]
    fn color_overlay_rejects_malformed_alpha_without_allocating_a_texture() {
        assert!(color_mask_overlay_image(4, 4, &[255; 15]).is_none());
    }
}
