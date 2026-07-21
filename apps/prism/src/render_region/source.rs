use std::{error::Error, fmt, fs::File, io::BufReader, path::Path};

use anyhow::{Context, Result, bail};
use image::{Rgba, RgbaImage};

use crate::{
    FontAsset, Layer, LayerKind, RasterSourceResolver, RegionRenderStats, RenderRegion,
    ResolvedRasterSource, TextTypography,
    raster_region::inspect_raster_region_source,
    shapes::ShapeSampler,
    text_render::{measure_text_with_typography, render_text_region},
};

const MAX_PNG_SCANLINE_BYTES: u64 = 64 * 1_024 * 1_024;
const MAX_SOURCE_STAGING_PIXELS: u64 = 4_096 * 4_096;
const MAX_ADJUSTED_STAGING_BYTES: u64 = 256 * 1_024 * 1_024;
const BLUR_RGBA_SURFACES: u64 = 12;

#[derive(Debug)]
struct SourceReadError(anyhow::Error);

impl fmt::Display for SourceReadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, formatter)
    }
}

impl Error for SourceReadError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.0.as_ref())
    }
}

#[derive(Debug)]
struct DynSourceReadError(Box<dyn Error + Send + Sync + 'static>);

impl fmt::Display for DynSourceReadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self.0.as_ref(), formatter)
    }
}

impl Error for DynSourceReadError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.0.as_ref())
    }
}

pub(super) fn layer_supports_region_reads(
    layer: &Layer,
    raster_sources: Option<&dyn RasterSourceResolver>,
) -> bool {
    match &layer.kind {
        LayerKind::Raster { path, .. } => raster_sources.map_or_else(
            || {
                inspect_raster_region_source(path)
                    .is_ok_and(|source| source.info.supports_region_reads_now())
            },
            |sources| {
                sources
                    .resolve(path)
                    .is_some_and(|source| source.source().info().supports_region_reads_now())
            },
        ),
        LayerKind::Text { .. } | LayerKind::Rectangle { .. } | LayerKind::Ellipse { .. } => true,
    }
}

pub(super) enum SourceDescriptor<'a> {
    RasterPath {
        path: &'a Path,
        dimensions: (u32, u32),
        adjustments: &'a spectrum_imaging::Adjustments,
    },
    RasterProvider {
        source: ResolvedRasterSource,
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
        raster_sources: Option<&dyn RasterSourceResolver>,
    ) -> Result<Self> {
        match &render_layer.kind {
            LayerKind::Raster { path, .. } => {
                if let Some(source) = raster_sources.and_then(|sources| sources.resolve(path)) {
                    let dimensions = {
                        let descriptor = &source.source().info().descriptor;
                        (descriptor.width, descriptor.height)
                    };
                    return Ok(Self::RasterProvider {
                        dimensions,
                        source,
                        adjustments: &render_layer.adjustments,
                    });
                }
                if raster_sources.is_some() {
                    bail!("raster source is not ready in the resolver snapshot");
                }
                Ok(Self::RasterPath {
                    dimensions: image::image_dimensions(path).with_context(|| {
                        format!("could not inspect layer source {}", path.display())
                    })?,
                    path,
                    adjustments: &render_layer.adjustments,
                })
            }
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

    pub(super) fn dimensions(&self) -> Result<(u32, u32)> {
        let base = self.base_dimensions();
        let dimensions =
            spectrum_imaging::adjusted_image_dimensions(base.0, base.1, self.adjustments())
                .context("Prism layer source must have positive dimensions")?;
        if dimensions.0 == 0 || dimensions.1 == 0 {
            bail!("Prism adjusted layer source must have positive dimensions");
        }
        Ok(dimensions)
    }

    pub(super) fn is_unadjusted_shape(&self) -> bool {
        matches!(self, Self::Shape { adjustments, .. } if **adjustments == spectrum_imaging::Adjustments::default())
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

        let base_dimensions = self.base_dimensions();
        let adjusted_staging_limit = adjusted_staging_pixel_limit(self.adjustments());
        let (image, adjusted_staging_pixels) =
            spectrum_imaging::render_image_region_at_source_resolution_bounded(
                base_dimensions.0,
                base_dimensions.1,
                self.adjustments().clone(),
                adjusted_region.into(),
                adjusted_staging_limit,
                |requested| {
                    self.stage_base(requested.into(), stats)
                        .map_err(SourceReadError)
                },
            )
            .map_err(anyhow::Error::new)?;
        stats.adjusted_staging_pixels = stats
            .adjusted_staging_pixels
            .saturating_add(adjusted_staging_pixels);
        stats.max_adjusted_staging_pixels = stats
            .max_adjusted_staging_pixels
            .max(adjusted_staging_pixels);
        Ok(SampleSource::Pixels {
            image,
            region: adjusted_region,
        })
    }

    fn base_dimensions(&self) -> (u32, u32) {
        match self {
            Self::RasterPath { dimensions, .. }
            | Self::RasterProvider { dimensions, .. }
            | Self::Text { dimensions, .. } => *dimensions,
            Self::Shape { sampler, .. } => sampler.dimensions(),
        }
    }

    fn adjustments(&self) -> &spectrum_imaging::Adjustments {
        match self {
            Self::RasterPath { adjustments, .. }
            | Self::RasterProvider { adjustments, .. }
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
            Self::RasterPath {
                path, dimensions, ..
            } => stage_png(path, *dimensions, region),
            Self::RasterProvider { source, .. } => {
                let image = source
                    .source()
                    .read_exact_region(region.into())
                    .map_err(|error| anyhow::Error::new(DynSourceReadError(error)))?;
                if image.dimensions() != (region.width, region.height) {
                    bail!(
                        "raster provider returned {}x{} pixels for a requested {}x{} region",
                        image.width(),
                        image.height(),
                        region.width,
                        region.height
                    );
                }
                Ok(image)
            }
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

fn adjusted_staging_pixel_limit(adjustments: &spectrum_imaging::Adjustments) -> u64 {
    // image::blur can retain the input plus eleven RGBA-sized working/output
    // surfaces; spot repair retains one immutable RGBA source beside its output.
    // Keep known imaging-owned adjusted intermediates inside the same 256 MiB
    // envelope as the legacy fallback while retaining the 4096-square ceiling.
    let surfaces = if adjustments.noise_reduction > 0.0 || adjustments.sharpening > 0.0 {
        BLUR_RGBA_SURFACES
    } else if adjustments.spots.is_empty() {
        1
    } else {
        2
    };
    MAX_SOURCE_STAGING_PIXELS.min(MAX_ADJUSTED_STAGING_BYTES / (4 * surfaces))
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

    pub(super) fn alpha(&self, x: u32, y: u32) -> u8 {
        match self {
            Self::Constant(pixel) => pixel[3],
            Self::Pixels { image, region } => image.get_pixel(x - region.x, y - region.y)[3],
            Self::Shape(sampler) => sampler.alpha(x, y),
        }
    }

    pub(super) fn supports_unstaged_alpha_tile(&self) -> bool {
        matches!(self, Self::Constant(_) | Self::Shape(_))
    }
}

pub(super) fn sample_triangle_resize_alpha(
    source_pixels: &SampleSource<'_>,
    source: (u32, u32),
    output: (u32, u32),
    coordinate: (u32, u32),
) -> u8 {
    if source == output {
        return source_pixels.alpha(coordinate.0, coordinate.1);
    }
    let x_weights = triangle_weights(source.0, output.0, coordinate.0);
    let y_weights = triangle_weights(source.1, output.1, coordinate.1);
    let mut horizontal = 0.0_f32;
    for source_x in x_weights.start..x_weights.end {
        let mut vertical = 0.0_f32;
        for source_y in y_weights.start..y_weights.end {
            let weight =
                triangle_weight(source_y, y_weights.center, y_weights.scale) / y_weights.sum;
            vertical += f32::from(source_pixels.alpha(source_x, source_y)) * weight;
        }
        let weight = triangle_weight(source_x, x_weights.center, x_weights.scale) / x_weights.sum;
        horizontal += vertical * weight;
    }
    horizontal.round().clamp(0.0, 255.0) as u8
}

pub(super) fn sample_triangle_resize(
    source_pixels: &SampleSource<'_>,
    source: (u32, u32),
    output: (u32, u32),
    coordinate: (u32, u32),
) -> [u8; 4] {
    if let SampleSource::Constant(pixel) = source_pixels {
        return *pixel;
    }
    if source == output {
        return source_pixels.pixel(coordinate.0, coordinate.1);
    }
    let x_weights = triangle_weights(source.0, output.0, coordinate.0);
    let y_weights = triangle_weights(source.1, output.1, coordinate.1);
    let mut horizontal = [0.0_f32; 4];
    for source_x in x_weights.start..x_weights.end {
        let mut vertical = [0.0_f32; 4];
        for source_y in y_weights.start..y_weights.end {
            let pixel = source_pixels.pixel(source_x, source_y);
            let weight =
                triangle_weight(source_y, y_weights.center, y_weights.scale) / y_weights.sum;
            for channel in 0..4 {
                vertical[channel] += f32::from(pixel[channel]) * weight;
            }
        }
        let weight = triangle_weight(source_x, x_weights.center, x_weights.scale) / x_weights.sum;
        for channel in 0..4 {
            horizontal[channel] += vertical[channel] * weight;
        }
    }
    horizontal.map(|channel| channel.round().clamp(0.0, 255.0) as u8)
}

#[derive(Clone, Copy)]
pub(super) struct TriangleWeights {
    pub(super) start: u32,
    pub(super) end: u32,
    center: f32,
    scale: f32,
    sum: f32,
}

pub(super) fn source_sample_bounds(source: u32, output: u32, coordinate: u32) -> TriangleWeights {
    if source == output {
        return TriangleWeights {
            start: coordinate,
            end: coordinate + 1,
            center: coordinate as f32,
            scale: 1.0,
            sum: 1.0,
        };
    }
    triangle_weights(source, output, coordinate)
}

fn triangle_weights(source: u32, output: u32, coordinate: u32) -> TriangleWeights {
    let ratio = source as f32 / output as f32;
    let scale = ratio.max(1.0);
    let input = (coordinate as f32 + 0.5) * ratio;
    let start = ((input - scale).floor() as i64).clamp(0, i64::from(source) - 1) as u32;
    let end = ((input + scale).ceil() as i64).clamp(i64::from(start) + 1, i64::from(source)) as u32;
    let center = input - 0.5;
    let sum = (start..end)
        .map(|sample| triangle_weight(sample, center, scale))
        .sum();
    TriangleWeights {
        start,
        end,
        center,
        scale,
        sum,
    }
}

fn triangle_weight(sample: u32, center: f32, scale: f32) -> f32 {
    (1.0 - ((sample as f32 - center) / scale).abs()).max(0.0)
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

fn stage_png(path: &Path, dimensions: (u32, u32), region: SourceRegion) -> Result<RgbaImage> {
    let mut reader = png_reader(path)?;
    if reader.info().interlaced {
        bail!("interlaced PNG does not support bounded region reads");
    }
    if reader.info().bit_depth == png::BitDepth::Sixteen {
        bail!("16-bit PNG does not support bounded region reads");
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
