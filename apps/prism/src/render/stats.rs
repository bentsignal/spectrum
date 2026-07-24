/// Allocation accounting for exact viewport compositing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RegionRenderStats {
    pub output_pixels: u64,
    pub source_staging_pixels: u64,
    pub source_staging_bytes: u64,
    pub max_source_staging_pixels: u64,
    pub adjusted_staging_pixels: u64,
    pub max_adjusted_staging_pixels: u64,
    pub full_source_pixels: u64,
    /// Bytes in the initial full-source fallback decode or rasterization.
    pub fallback_decode_bytes: u64,
    /// Conservative peak bytes estimated for one full-source fallback layer.
    pub fallback_peak_bytes: u64,
    /// Pixels materialized in full scaled and rotated fallback surfaces.
    pub transformed_surface_pixels: u64,
    /// Logical fixed-kernel alpha taps used for layer drop shadows.
    pub shadow_samples: u64,
    /// Actual source-alpha evaluations after bounded tile reuse.
    pub shadow_source_samples: u64,
    /// Cumulative one-byte alpha-tile pixels allocated for bounded shadows.
    pub shadow_alpha_tile_pixels: u64,
    pub shadow_alpha_tile_bytes: u64,
    /// Peak live alpha-tile allocation; tiles are processed one layer at a time.
    pub max_shadow_alpha_tile_pixels: u64,
    pub max_shadow_alpha_tile_bytes: u64,
}
