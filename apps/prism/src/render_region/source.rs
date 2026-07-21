use std::{fs::File, io::BufReader, path::Path};

use anyhow::{Context, Result, anyhow, bail};
use image::{Rgba, RgbaImage};

use crate::{
    FontAsset, Layer, LayerKind, RegionRenderStats, RenderRegion, TextTypography,
    shapes::ShapeSampler,
    text_render::{measure_text_with_typography, render_text_region},
};

const MAX_PNG_SCANLINE_BYTES: u64 = 64 * 1_024 * 1_024;
const MAX_SOURCE_STAGING_PIXELS: u64 = 4_096 * 4_096;

pub(super) fn layer_supports_region_reads(layer: &Layer) -> bool {
    match &layer.kind {
        LayerKind::Raster { path, .. } => png_supports_region_reads(path),
        LayerKind::Text { .. } | LayerKind::Rectangle { .. } | LayerKind::Ellipse { .. } => true,
    }
}

pub(super) enum SourceDescriptor<'a> {
    Raster {
        path: &'a Path,
        dimensions: (u32, u32),
        adjustments: &'a spectrum_imaging::Adjustments,
    },
    Text {
        text: &'a str,
        font_size: f32,
        color: [u8; 4],
        typography: &'a TextTypography,
        font_asset: Option<&'a FontAsset>,
        dimensions: (u32, u32),
        adjustments: &'a spectrum_imaging::Adjustments,
    },
    Shape {
        sampler: ShapeSampler<'a>,
        adjustments: &'a spectrum_imaging::Adjustments,
    },
}

impl<'a> SourceDescriptor<'a> {
    pub(super) fn new(
        render_layer: &'a Layer,
        shape_scale: [f32; 2],
        font_asset: Option<&'a FontAsset>,
    ) -> Result<Self> {
        match &render_layer.kind {
            LayerKind::Raster { path, .. } => Ok(Self::Raster {
                dimensions: image::image_dimensions(path).with_context(|| {
                    format!("could not inspect layer source {}", path.display())
                })?,
                path,
                adjustments: &render_layer.adjustments,
            }),
            LayerKind::Text {
                text,
                font_size,
                color,
                typography,
            } => Ok(Self::Text {
                text,
                font_size: *font_size,
                color: *color,
                typography,
                font_asset,
                dimensions: measure_text_with_typography(text, *font_size, typography, font_asset)?,
                adjustments: &render_layer.adjustments,
            }),
            LayerKind::Rectangle { .. } | LayerKind::Ellipse { .. } => Ok(Self::Shape {
                sampler: ShapeSampler::new(render_layer, shape_scale)?,
                adjustments: &render_layer.adjustments,
            }),
        }
    }

    pub(super) fn dimensions(&self) -> (u32, u32) {
        let base = self.base_dimensions();
        spectrum_imaging::adjusted_image_dimensions(base.0, base.1, self.adjustments())
            .expect("Prism layer sources always have positive dimensions")
    }

    pub(super) fn sample(
        &self,
        adjusted_region: SourceRegion,
        stats: &mut RegionRenderStats,
    ) -> Result<SampleSource<'a>> {
        if self.adjustments() == &spectrum_imaging::Adjustments::default()
            && let Self::Shape { sampler, .. } = self
        {
            return Ok(SampleSource::shape(*sampler));
        }
        if self.adjustments() == &spectrum_imaging::Adjustments::default() {
            let image = self.stage_base(adjusted_region, stats)?;
            return Ok(SampleSource::Pixels {
                image,
                region: adjusted_region,
            });
        }

        let adjusted_pixels = adjusted_region.pixel_count();
        stats.adjusted_staging_pixels = stats
            .adjusted_staging_pixels
            .saturating_add(adjusted_pixels);
        stats.max_adjusted_staging_pixels = stats.max_adjusted_staging_pixels.max(adjusted_pixels);
        let base_dimensions = self.base_dimensions();
        let image = spectrum_imaging::render_image_region_at_source_resolution(
            base_dimensions.0,
            base_dimensions.1,
            self.adjustments().clone(),
            adjusted_region.into(),
            |requested| {
                self.stage_base(requested.into(), stats)
                    .map_err(|error| format!("{error:#}"))
            },
        )
        .map_err(|error| anyhow!(error.to_string()))?;
        Ok(SampleSource::Pixels {
            image,
            region: adjusted_region,
        })
    }

    fn base_dimensions(&self) -> (u32, u32) {
        match self {
            Self::Raster { dimensions, .. } | Self::Text { dimensions, .. } => *dimensions,
            Self::Shape { sampler, .. } => sampler.dimensions(),
        }
    }

    fn adjustments(&self) -> &spectrum_imaging::Adjustments {
        match self {
            Self::Raster { adjustments, .. }
            | Self::Text { adjustments, .. }
            | Self::Shape { adjustments, .. } => adjustments,
        }
    }

    fn stage_base(&self, region: SourceRegion, stats: &mut RegionRenderStats) -> Result<RgbaImage> {
        let pixels = region.pixel_count();
        if pixels > MAX_SOURCE_STAGING_PIXELS {
            bail!("adjusted layer requires more than the bounded base-source staging budget");
        }
        stats.source_staging_pixels = stats.source_staging_pixels.saturating_add(pixels);
        stats.source_staging_bytes = stats
            .source_staging_bytes
            .saturating_add(pixels.saturating_mul(4));
        stats.max_source_staging_pixels = stats.max_source_staging_pixels.max(pixels);
        match self {
            Self::Raster {
                path, dimensions, ..
            } => stage_png(path, *dimensions, region),
            Self::Text {
                text,
                font_size,
                color,
                typography,
                font_asset,
                ..
            } => render_text_region(
                text,
                *font_size,
                *color,
                typography,
                *font_asset,
                region.into(),
            ),
            Self::Shape { sampler, .. } => {
                Ok(RgbaImage::from_fn(region.width, region.height, |x, y| {
                    sampler.pixel(region.x + x, region.y + y)
                }))
            }
        }
    }
}

pub(super) enum SampleSource<'a> {
    Constant([u8; 4]),
    Pixels {
        image: RgbaImage,
        region: SourceRegion,
    },
    Shape(ShapeSampler<'a>),
}

impl<'a> SampleSource<'a> {
    pub(super) fn shape(sampler: ShapeSampler<'a>) -> Self {
        sampler
            .uniform_pixel()
            .map_or(Self::Shape(sampler), |pixel| Self::Constant(pixel.0))
    }

    pub(super) fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        match self {
            Self::Constant(pixel) => *pixel,
            Self::Pixels { image, region } => image.get_pixel(x - region.x, y - region.y).0,
            Self::Shape(sampler) => sampler.pixel(x, y).0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct SourceRegion {
    pub(super) x: u32,
    pub(super) y: u32,
    pub(super) width: u32,
    pub(super) height: u32,
}

impl SourceRegion {
    pub(super) fn pixel_count(self) -> u64 {
        u64::from(self.width) * u64::from(self.height)
    }
}

impl From<SourceRegion> for RenderRegion {
    fn from(region: SourceRegion) -> Self {
        Self {
            x: region.x,
            y: region.y,
            width: region.width,
            height: region.height,
        }
    }
}

impl From<SourceRegion> for spectrum_imaging::PixelRegion {
    fn from(region: SourceRegion) -> Self {
        Self {
            x: region.x,
            y: region.y,
            width: region.width,
            height: region.height,
        }
    }
}

impl From<spectrum_imaging::PixelRegion> for SourceRegion {
    fn from(region: spectrum_imaging::PixelRegion) -> Self {
        Self {
            x: region.x,
            y: region.y,
            width: region.width,
            height: region.height,
        }
    }
}

fn png_supports_region_reads(path: &Path) -> bool {
    png_reader(path)
        .ok()
        .is_some_and(|reader| !reader.info().interlaced)
}

fn stage_png(path: &Path, dimensions: (u32, u32), region: SourceRegion) -> Result<RgbaImage> {
    let mut reader = png_reader(path)?;
    if reader.info().interlaced {
        bail!("interlaced PNG does not support bounded region reads");
    }
    if (reader.info().width, reader.info().height) != dimensions {
        bail!("PNG dimensions changed while staging {}", path.display());
    }
    let (color_type, bit_depth) = reader.output_color_type();
    if bit_depth != png::BitDepth::Eight {
        bail!("PNG staging requires 8-bit transformed rows");
    }
    let channels = png_channels(color_type);
    let row_bytes = u64::from(dimensions.0) * channels as u64;
    if row_bytes > MAX_PNG_SCANLINE_BYTES {
        bail!("PNG scanline exceeds the bounded source staging budget");
    }
    let mut output = RgbaImage::new(region.width, region.height);
    for source_y in 0..region.y + region.height {
        let row = reader
            .next_row()
            .with_context(|| format!("could not decode PNG row {}", path.display()))?
            .context("PNG ended before the requested source region")?;
        if source_y < region.y {
            continue;
        }
        for source_x in region.x..region.x + region.width {
            let offset = source_x as usize * channels;
            output.put_pixel(
                source_x - region.x,
                source_y - region.y,
                Rgba(png_pixel(
                    &row.data()[offset..offset + channels],
                    color_type,
                )),
            );
        }
    }
    Ok(output)
}

fn png_reader(path: &Path) -> Result<png::Reader<BufReader<File>>> {
    let file = File::open(path).with_context(|| format!("could not open {}", path.display()))?;
    let mut decoder = png::Decoder::new_with_limits(
        BufReader::new(file),
        png::Limits {
            bytes: MAX_PNG_SCANLINE_BYTES as usize,
        },
    );
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    decoder
        .read_info()
        .with_context(|| format!("could not decode PNG header {}", path.display()))
}

fn png_channels(color_type: png::ColorType) -> usize {
    match color_type {
        png::ColorType::Grayscale => 1,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        png::ColorType::Indexed => unreachable!("EXPAND removes indexed PNG output"),
    }
}

fn png_pixel(bytes: &[u8], color_type: png::ColorType) -> [u8; 4] {
    match color_type {
        png::ColorType::Grayscale => [bytes[0], bytes[0], bytes[0], 255],
        png::ColorType::GrayscaleAlpha => [bytes[0], bytes[0], bytes[0], bytes[1]],
        png::ColorType::Rgb => [bytes[0], bytes[1], bytes[2], 255],
        png::ColorType::Rgba => [bytes[0], bytes[1], bytes[2], bytes[3]],
        png::ColorType::Indexed => unreachable!("EXPAND removes indexed PNG output"),
    }
}
