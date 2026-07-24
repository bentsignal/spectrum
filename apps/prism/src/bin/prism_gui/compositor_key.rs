use super::*;
use std::sync::Arc;

const MAX_FALLBACK_OFFSCREEN_DIMENSION: u32 = 4_096;

#[derive(Clone, Debug, PartialEq)]
pub(super) struct CompositePreviewKey {
    pub(super) tab_id: u64,
    pub(super) generation: u64,
    pub(super) document: Document,
    pub(super) scale_sixty_fourths: u32,
    pub(super) region: prism_core::RenderRegion,
    pub(super) raster_mode: RasterRenderMode,
    pub(super) progressive_brush: Option<ProgressiveBrushPreview>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ProgressiveBrushPreview {
    pub(crate) gesture_id: u64,
    pub(crate) target_layer_id: u64,
    pub(crate) sample_count: usize,
    pub(crate) mode: prism_core::BrushMode,
}

pub(crate) struct CompositePreviewRequest<'a> {
    pub(crate) tab_id: u64,
    pub(crate) document: &'a Document,
    pub(crate) geometry: CanvasGeometry,
    pub(crate) physical_pixels_per_point: f32,
    pub(crate) raster_sources: Arc<RasterSourceSnapshot>,
    pub(crate) progressive_brush: Option<ProgressiveBrushPreview>,
}

impl CompositePreviewKey {
    #[cfg(test)]
    pub(super) fn new_with_sources(
        tab_id: u64,
        generation: u64,
        document: &Document,
        geometry: CanvasGeometry,
        physical_pixels_per_point: f32,
        raster_sources: &RasterSourceSnapshot,
    ) -> Option<Self> {
        Self::new_with_sources_and_brush(
            tab_id,
            generation,
            document,
            geometry,
            physical_pixels_per_point,
            raster_sources,
            None,
        )
    }

    pub(super) fn new_with_sources_and_brush(
        tab_id: u64,
        generation: u64,
        document: &Document,
        geometry: CanvasGeometry,
        physical_pixels_per_point: f32,
        raster_sources: &RasterSourceSnapshot,
        progressive_brush: Option<ProgressiveBrushPreview>,
    ) -> Option<Self> {
        let display_scale = geometry.pixels_per_point * physical_pixels_per_point;
        let requested_units = ((display_scale.max(1.0 / 64.0) * 64.0).ceil() as u32).max(1);
        let raster_mode = raster_sources.render_mode(document);
        let scale_sixty_fourths = if !matches!(raster_mode, RasterRenderMode::FallbackCapped) {
            requested_units
        } else {
            let longest = document.width.max(document.height).max(1) as f32;
            let maximum_units = ((MAX_FALLBACK_OFFSCREEN_DIMENSION as f32 / longest) * 64.0)
                .floor()
                .max(1.0) as u32;
            requested_units.min(maximum_units)
        };
        let scale = scale_sixty_fourths as f32 / 64.0;
        let region = visible_render_region(geometry, document, scale)?;
        let mut document = document.clone();
        document.selected = None;
        Some(Self {
            tab_id,
            generation,
            document,
            scale_sixty_fourths,
            region,
            raster_mode,
            progressive_brush,
        })
    }

    pub(super) fn scale(&self) -> f32 {
        self.scale_sixty_fourths as f32 / 64.0
    }

    #[cfg(test)]
    pub(super) fn new(
        tab_id: u64,
        generation: u64,
        document: &Document,
        geometry: CanvasGeometry,
        physical_pixels_per_point: f32,
    ) -> Option<Self> {
        Self::new_with_sources(
            tab_id,
            generation,
            document,
            geometry,
            physical_pixels_per_point,
            &RasterSourceSnapshot::empty(),
        )
    }
}
