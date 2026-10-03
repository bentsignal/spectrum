//! Where a layer's styles draw within a render region, and the layer alpha
//! they are computed from.
use crate::{Layer, LayerStyle, RenderRegion, layer_effects::AlphaTile};

use super::{
    CanvasIntersection, MAX_SOURCE_STAGING_PIXELS, SampleSource, SamplingGeometry,
    sample_output_alpha,
};

/// The alpha tile a layer's styles need for one region, in output pixels.
#[derive(Clone, Copy)]
pub(super) struct EffectsTileBounds {
    left: i64,
    top: i64,
    right: i64,
    bottom: i64,
}

/// The tile covering what the styles draw inside `region`, plus the context
/// around it they read; `None` without styles or when nothing shows.
pub(super) fn effects_tile_bounds(
    geometry: &SamplingGeometry,
    region: RenderRegion,
    style: &LayerStyle,
) -> Option<EffectsTileBounds> {
    if !crate::layer_effects::has_effects(style) {
        return None;
    }
    let reach = crate::layer_effects::effects_reach(style);
    let (width, height) = (
        i64::from(geometry.output_width),
        i64::from(geometry.output_height),
    );
    // What the styles can draw, in output pixels, clipped to the region.
    let left = (-reach).max(i64::from(region.x) - geometry.origin_x);
    let top = (-reach).max(i64::from(region.y) - geometry.origin_y);
    let right = (width + reach).min(i64::from(region.x + region.width) - geometry.origin_x);
    let bottom = (height + reach).min(i64::from(region.y + region.height) - geometry.origin_y);
    if right <= left || bottom <= top {
        return None;
    }
    let bounds = EffectsTileBounds {
        left: (left - reach).max(-reach),
        top: (top - reach).max(-reach),
        right: (right + reach).min(width + reach),
        bottom: (bottom + reach).min(height + reach),
    };
    let pixels = (bounds.right - bounds.left) as u64 * (bounds.bottom - bounds.top) as u64;
    (pixels <= MAX_SOURCE_STAGING_PIXELS).then_some(bounds)
}

impl EffectsTileBounds {
    /// The part of the tile the layer itself covers, in canvas pixels.
    pub(super) fn within_output(self, geometry: &SamplingGeometry) -> Option<CanvasIntersection> {
        let left = self.left.max(0);
        let top = self.top.max(0);
        let right = self.right.min(i64::from(geometry.output_width));
        let bottom = self.bottom.min(i64::from(geometry.output_height));
        (right > left && bottom > top).then_some(CanvasIntersection {
            left: geometry.origin_x + left,
            top: geometry.origin_y + top,
            right: geometry.origin_x + right,
            bottom: geometry.origin_y + bottom,
        })
    }

    pub(super) fn sample(
        self,
        source: &SampleSource<'_>,
        geometry: &SamplingGeometry,
        layer: &Layer,
    ) -> AlphaTile {
        let width = (self.right - self.left) as usize;
        let height = (self.bottom - self.top) as usize;
        let mut alpha = Vec::with_capacity(width * height);
        for y in self.top..self.bottom {
            for x in self.left..self.right {
                alpha.push(sample_output_alpha(source, geometry, layer, x, y));
            }
        }
        AlphaTile {
            left: self.left,
            top: self.top,
            width,
            height,
            alpha,
        }
    }
}
