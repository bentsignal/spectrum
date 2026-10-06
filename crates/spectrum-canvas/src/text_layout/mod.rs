//! Text layout: HarfBuzz shaping, line breaking, and glyph rendering.

mod font_resolver;
mod geometry;
mod glyph_raster;
mod script;
mod shaped;

use anyhow::Result;
use image::RgbaImage;

pub use geometry::TextGeometry;

use crate::{FontAsset, RenderRegion, TextTypography};

pub fn measure_text(text: &str, font_size: f32) -> Result<(u32, u32)> {
    measure_text_with_typography(text, font_size, &TextTypography::default(), None)
}

pub fn measure_text_geometry(text: &str, font_size: f32) -> Result<TextGeometry> {
    measure_text_geometry_with_typography(text, font_size, &TextTypography::default(), None)
}

pub fn measure_text_with_typography(
    text: &str,
    font_size: f32,
    typography: &TextTypography,
    font_asset: Option<&FontAsset>,
) -> Result<(u32, u32)> {
    let geometry = measure_text_geometry_with_typography(text, font_size, typography, font_asset)?;
    Ok((geometry.width, geometry.height))
}

pub fn measure_text_geometry_with_typography(
    text: &str,
    font_size: f32,
    typography: &TextTypography,
    font_asset: Option<&FontAsset>,
) -> Result<TextGeometry> {
    shaped::measure(text, font_size, typography, font_asset)
}

pub(crate) fn render_text(
    text: &str,
    font_size: f32,
    color: [u8; 4],
    typography: &TextTypography,
    font_asset: Option<&FontAsset>,
) -> Result<RgbaImage> {
    shaped::render(text, font_size, color, typography, font_asset)
}

pub(crate) fn render_text_region(
    text: &str,
    font_size: f32,
    color: [u8; 4],
    typography: &TextTypography,
    font_asset: Option<&FontAsset>,
    region: RenderRegion,
) -> Result<RgbaImage> {
    shaped::render_region(text, font_size, color, typography, font_asset, region)
}
