use std::{error::Error, fmt};

use image::{DynamicImage, Rgba, RgbaImage};

use super::{apply_color_adjustments_region, apply_unsharp, blend_images};
use crate::Adjustments;

/// A pixel-space rectangle in an adjusted image or its source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PixelRegion {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// Failure from rendering an adjusted image region.
#[derive(Debug)]
pub enum RegionRenderError<E> {
    InvalidSourceDimensions,
    InvalidRegion(&'static str),
    UnsupportedSpots,
    Source(E),
    SourceRegionDimensions {
        requested: PixelRegion,
        actual: (u32, u32),
    },
}

impl<E: fmt::Display> fmt::Display for RegionRenderError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSourceDimensions => {
                formatter.write_str("adjusted image source must have positive dimensions")
            }
            Self::InvalidRegion(message) => formatter.write_str(message),
            Self::UnsupportedSpots => formatter
                .write_str("spot removal is not supported by bounded adjusted-region rendering"),
            Self::Source(error) => {
                write!(formatter, "could not read adjusted image source: {error}")
            }
            Self::SourceRegionDimensions { requested, actual } => write!(
                formatter,
                "source reader returned {}x{} for requested {}x{} region",
                actual.0, actual.1, requested.width, requested.height
            ),
        }
    }
}

impl<E: Error + 'static> Error for RegionRenderError<E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Source(error) => Some(error),
            _ => None,
        }
    }
}

/// Renders an exact crop of an adjusted image from one bounded source read.
///
/// The source callback receives the base-image rectangle needed for `region`,
/// including finite filter halos. It must return pixels whose local origin is
/// the requested rectangle's `(x, y)`. Development geometry and pixel effects
/// otherwise follow [`super::render_image`] semantics. Spot removal is rejected
/// until its non-local repair sampling has a bounded exact implementation.
pub fn render_image_region<E, S>(
    source_width: u32,
    source_height: u32,
    adjustments: Adjustments,
    region: PixelRegion,
    mut read_source_region: S,
) -> Result<RgbaImage, RegionRenderError<E>>
where
    S: FnMut(PixelRegion) -> Result<RgbaImage, E>,
{
    if source_width == 0 || source_height == 0 {
        return Err(RegionRenderError::InvalidSourceDimensions);
    }
    let adjustments = adjustments.sanitized();
    if !adjustments.spots.is_empty() {
        return Err(RegionRenderError::UnsupportedSpots);
    }
    let geometry = AdjustedGeometry::new(source_width, source_height, &adjustments);
    validate_region(region, geometry.output_width, geometry.output_height)?;

    let sharpen_region = expand_region(
        region,
        u32::from(adjustments.sharpening > 0.0) * 2,
        geometry.output_width,
        geometry.output_height,
    );
    let noise_region = expand_region(
        sharpen_region,
        u32::from(adjustments.noise_reduction > 0.0) * 4,
        geometry.output_width,
        geometry.output_height,
    );
    let source_region = geometry.required_source_region(noise_region);
    let source = read_source_region(source_region).map_err(RegionRenderError::Source)?;
    if source.dimensions() != (source_region.width, source_region.height) {
        return Err(RegionRenderError::SourceRegionDimensions {
            requested: source_region,
            actual: source.dimensions(),
        });
    }

    let mut pixels = geometry.materialize(noise_region, source_region, &source);
    if adjustments.noise_reduction > 0.0 {
        let blurred = DynamicImage::ImageRgba8(pixels.clone())
            .blur(1.6)
            .to_rgba8();
        pixels = blend_images(
            &pixels,
            &blurred,
            adjustments.noise_reduction / 100.0 * 0.75,
        );
    }
    pixels = crop_region(&pixels, noise_region, sharpen_region);
    apply_color_adjustments_region(
        &mut pixels,
        &adjustments,
        sharpen_region.x,
        sharpen_region.y,
        geometry.output_width,
        geometry.output_height,
    );
    if adjustments.sharpening > 0.0 {
        let blurred = DynamicImage::ImageRgba8(pixels.clone())
            .blur(1.1)
            .to_rgba8();
        apply_unsharp(&mut pixels, &blurred, adjustments.sharpening / 100.0 * 1.8);
    }
    Ok(crop_region(&pixels, sharpen_region, region))
}

/// Dimensions after development rotation, straighten, and crop.
pub fn adjusted_image_dimensions(
    source_width: u32,
    source_height: u32,
    adjustments: &Adjustments,
) -> (u32, u32) {
    let adjustments = adjustments.clone().sanitized();
    let geometry = AdjustedGeometry::new(source_width.max(1), source_height.max(1), &adjustments);
    (geometry.output_width, geometry.output_height)
}

#[derive(Clone, Copy)]
struct AdjustedGeometry {
    source_width: u32,
    source_height: u32,
    oriented_width: u32,
    oriented_height: u32,
    output_width: u32,
    output_height: u32,
    crop_x: u32,
    crop_y: u32,
    rotation: i32,
    flip_horizontal: bool,
    flip_vertical: bool,
    straighten: f32,
}

impl AdjustedGeometry {
    fn new(source_width: u32, source_height: u32, adjustments: &Adjustments) -> Self {
        let (oriented_width, oriented_height) = if matches!(adjustments.rotation, 90 | 270) {
            (source_height, source_width)
        } else {
            (source_width, source_height)
        };
        let (crop_x, crop_y, output_width, output_height) =
            adjustments
                .crop
                .map_or((0, 0, oriented_width, oriented_height), |crop| {
                    let x = (crop.x * oriented_width as f32).round() as u32;
                    let y = (crop.y * oriented_height as f32).round() as u32;
                    let width = (crop.width * oriented_width as f32).round().max(1.0) as u32;
                    let height = (crop.height * oriented_height as f32).round().max(1.0) as u32;
                    (
                        x.min(oriented_width - 1),
                        y.min(oriented_height - 1),
                        width.min(oriented_width - x),
                        height.min(oriented_height - y),
                    )
                });
        Self {
            source_width,
            source_height,
            oriented_width,
            oriented_height,
            output_width,
            output_height,
            crop_x,
            crop_y,
            rotation: adjustments.rotation,
            flip_horizontal: adjustments.flip_horizontal,
            flip_vertical: adjustments.flip_vertical,
            straighten: adjustments.straighten,
        }
    }

    fn required_source_region(self, region: PixelRegion) -> PixelRegion {
        let mut bounds = SourceBounds::empty();
        for y in region.y..region.y + region.height {
            for x in region.x..region.x + region.width {
                self.visit_source_samples(x, y, |source_x, source_y| {
                    bounds.include(source_x, source_y);
                });
            }
        }
        bounds.region()
    }

    fn materialize(
        self,
        region: PixelRegion,
        source_region: PixelRegion,
        source: &RgbaImage,
    ) -> RgbaImage {
        RgbaImage::from_fn(region.width, region.height, |x, y| {
            self.pixel(region.x + x, region.y + y, source_region, source)
        })
    }

    fn pixel(self, x: u32, y: u32, source_region: PixelRegion, source: &RgbaImage) -> Rgba<u8> {
        let x = x + self.crop_x;
        let y = y + self.crop_y;
        let Some((sample_x, sample_y)) = self.straightened_coordinates(x, y) else {
            let (source_x, source_y) = self.source_coordinates(x, y);
            return local_pixel(source, source_region, source_x, source_y);
        };
        let x0 = sample_x.floor() as u32;
        let y0 = sample_y.floor() as u32;
        let x1 = (x0 + 1).min(self.oriented_width - 1);
        let y1 = (y0 + 1).min(self.oriented_height - 1);
        let tx = sample_x - x0 as f32;
        let ty = sample_y - y0 as f32;
        let sample = |oriented_x, oriented_y| {
            let (source_x, source_y) = self.source_coordinates(oriented_x, oriented_y);
            local_pixel(source, source_region, source_x, source_y)
        };
        let samples = [
            sample(x0, y0),
            sample(x1, y0),
            sample(x0, y1),
            sample(x1, y1),
        ];
        let mut output = [0; 4];
        for channel in 0..4 {
            let top = samples[0][channel] as f32 * (1.0 - tx) + samples[1][channel] as f32 * tx;
            let bottom = samples[2][channel] as f32 * (1.0 - tx) + samples[3][channel] as f32 * tx;
            output[channel] = (top * (1.0 - ty) + bottom * ty + 0.5) as u8;
        }
        Rgba(output)
    }

    fn visit_source_samples(self, x: u32, y: u32, mut visit: impl FnMut(u32, u32)) {
        let x = x + self.crop_x;
        let y = y + self.crop_y;
        let Some((sample_x, sample_y)) = self.straightened_coordinates(x, y) else {
            let (source_x, source_y) = self.source_coordinates(x, y);
            visit(source_x, source_y);
            return;
        };
        let x0 = sample_x.floor() as u32;
        let y0 = sample_y.floor() as u32;
        let x1 = (x0 + 1).min(self.oriented_width - 1);
        let y1 = (y0 + 1).min(self.oriented_height - 1);
        for (oriented_x, oriented_y) in [(x0, y0), (x1, y0), (x0, y1), (x1, y1)] {
            let (source_x, source_y) = self.source_coordinates(oriented_x, oriented_y);
            visit(source_x, source_y);
        }
    }

    fn straightened_coordinates(self, x: u32, y: u32) -> Option<(f32, f32)> {
        if self.straighten.abs() <= 0.01 {
            return None;
        }
        let radians = self.straighten.to_radians();
        let (sin, cos) = radians.sin_cos();
        let aspect = self.oriented_width as f32 / self.oriented_height.max(1) as f32;
        let zoom = (cos.abs() + aspect * sin.abs())
            .max(cos.abs() + sin.abs() / aspect)
            .max(1.0);
        let center_x = (self.oriented_width as f32 - 1.0) * 0.5;
        let center_y = (self.oriented_height as f32 - 1.0) * 0.5;
        let dx = (x as f32 - center_x) / zoom;
        let dy = (y as f32 - center_y) / zoom;
        Some((
            (cos * dx + sin * dy + center_x)
                .clamp(0.0, self.oriented_width.saturating_sub(1) as f32),
            (-sin * dx + cos * dy + center_y)
                .clamp(0.0, self.oriented_height.saturating_sub(1) as f32),
        ))
    }

    fn source_coordinates(self, mut x: u32, mut y: u32) -> (u32, u32) {
        if self.flip_vertical {
            y = self.oriented_height - y - 1;
        }
        if self.flip_horizontal {
            x = self.oriented_width - x - 1;
        }
        match self.rotation {
            90 => (y, self.source_height - x - 1),
            180 => (self.source_width - x - 1, self.source_height - y - 1),
            270 => (self.source_width - y - 1, x),
            _ => (x, y),
        }
    }
}

struct SourceBounds {
    left: u32,
    top: u32,
    right: u32,
    bottom: u32,
}

impl SourceBounds {
    fn empty() -> Self {
        Self {
            left: u32::MAX,
            top: u32::MAX,
            right: 0,
            bottom: 0,
        }
    }

    fn include(&mut self, x: u32, y: u32) {
        self.left = self.left.min(x);
        self.top = self.top.min(y);
        self.right = self.right.max(x + 1);
        self.bottom = self.bottom.max(y + 1);
    }

    fn region(self) -> PixelRegion {
        PixelRegion {
            x: self.left,
            y: self.top,
            width: self.right - self.left,
            height: self.bottom - self.top,
        }
    }
}

fn local_pixel(source: &RgbaImage, region: PixelRegion, x: u32, y: u32) -> Rgba<u8> {
    *source.get_pixel(x - region.x, y - region.y)
}

fn validate_region<E>(
    region: PixelRegion,
    width: u32,
    height: u32,
) -> Result<(), RegionRenderError<E>> {
    if region.width == 0 || region.height == 0 {
        return Err(RegionRenderError::InvalidRegion(
            "adjusted image region must have positive dimensions",
        ));
    }
    let right = region
        .x
        .checked_add(region.width)
        .ok_or(RegionRenderError::InvalidRegion(
            "adjusted image region overflows horizontally",
        ))?;
    let bottom = region
        .y
        .checked_add(region.height)
        .ok_or(RegionRenderError::InvalidRegion(
            "adjusted image region overflows vertically",
        ))?;
    if right > width || bottom > height {
        return Err(RegionRenderError::InvalidRegion(
            "adjusted image region exceeds the adjusted image",
        ));
    }
    Ok(())
}

fn expand_region(region: PixelRegion, radius: u32, width: u32, height: u32) -> PixelRegion {
    let x = region.x.saturating_sub(radius);
    let y = region.y.saturating_sub(radius);
    let right = region
        .x
        .saturating_add(region.width)
        .saturating_add(radius)
        .min(width);
    let bottom = region
        .y
        .saturating_add(region.height)
        .saturating_add(radius)
        .min(height);
    PixelRegion {
        x,
        y,
        width: right - x,
        height: bottom - y,
    }
}

fn crop_region(image: &RgbaImage, outer: PixelRegion, inner: PixelRegion) -> RgbaImage {
    image::imageops::crop_imm(
        image,
        inner.x - outer.x,
        inner.y - outer.y,
        inner.width,
        inner.height,
    )
    .to_image()
}
