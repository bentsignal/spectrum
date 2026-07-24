use crate::{
    Layer,
    effects::{DROP_SHADOW_KERNEL, DROP_SHADOW_KERNEL_TAPS},
};

use super::{
    CanvasIntersection, MAX_SOURCE_STAGING_PIXELS, SampleSource, SamplingGeometry,
    sample_output_alpha, shadow_alpha_bounds,
};

/// Unique source-alpha samples needed by one visible shadow region.
///
/// The fixed shadow kernel revisits most coordinates across neighboring output
/// pixels. Materializing this one-byte bounded tile preserves the exact alpha
/// oracle while evaluating adjusted geometry and masks only once per unique
/// coordinate. Oversized tiles fall back to direct sampling.
pub(super) struct ShadowAlphaTile {
    left: i64,
    top: i64,
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

impl ShadowAlphaTile {
    pub(super) fn pixel_count(&self) -> u64 {
        u64::from(self.width) * u64::from(self.height)
    }

    pub(super) fn bounded(
        source: &SampleSource<'_>,
        geometry: &SamplingGeometry,
        layer: &Layer,
        intersection: CanvasIntersection,
        shadow: crate::DropShadow,
    ) -> Option<Self> {
        let bounds = shadow_alpha_bounds(geometry, intersection, shadow)?;
        if bounds.pixel_count() > MAX_SOURCE_STAGING_PIXELS {
            return None;
        }
        let mut pixels = Vec::new();
        pixels
            .try_reserve_exact(bounds.pixel_count() as usize)
            .ok()?;
        for y in bounds.top..bounds.bottom {
            for x in bounds.left..bounds.right {
                pixels.push(sample_output_alpha(source, geometry, layer, x, y));
            }
        }
        Some(Self {
            left: bounds.left,
            top: bounds.top,
            width: u32::try_from(bounds.right - bounds.left).ok()?,
            height: u32::try_from(bounds.bottom - bounds.top).ok()?,
            pixels,
        })
    }

    #[inline]
    fn alpha(&self, x: i64, y: i64) -> u8 {
        let local_x = x - self.left;
        let local_y = y - self.top;
        if local_x < 0
            || local_y < 0
            || local_x >= i64::from(self.width)
            || local_y >= i64::from(self.height)
        {
            return 0;
        }
        self.pixels[local_y as usize * self.width as usize + local_x as usize]
    }

    #[inline]
    pub(super) fn filtered_alpha(&self, center_x: i64, center_y: i64, radius: f32) -> u8 {
        if radius < 0.5 {
            return self.alpha(center_x, center_y);
        }
        let mut weighted_alpha = 0_u32;
        let mut total_weight = 0_u32;
        for (unit_x, unit_y, weight) in DROP_SHADOW_KERNEL {
            let x = center_x + (unit_x * radius).round() as i64;
            let y = center_y + (unit_y * radius).round() as i64;
            weighted_alpha += u32::from(self.alpha(x, y)) * weight;
            total_weight += weight;
        }
        debug_assert_eq!(DROP_SHADOW_KERNEL_TAPS, DROP_SHADOW_KERNEL.len() as u64);
        (weighted_alpha / total_weight) as u8
    }
}
