use std::{error::Error, fmt};

use image::{DynamicImage, Rgba, RgbaImage};

use super::{apply_color_adjustments_region, apply_unsharp, blend_images};
use crate::{Adjustments, SpotRemoval};

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
    ExceedsStagingPixelLimit,
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
            Self::ExceedsStagingPixelLimit => {
                formatter.write_str("adjusted image region exceeds the staging pixel limit")
            }
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

/// Renders an exact source-resolution crop from one bounded source read.
///
/// The source callback receives the base-image rectangle needed for `region`,
/// including finite filter halos. It must return pixels whose local origin is
/// the requested rectangle's `(x, y)`.
///
/// This matches [`super::render_image`] called with `RenderOptions::default()`.
/// It deliberately has no `max_size` equivalent: callers that need a resized
/// preview must resize separately or use the full-image renderer. Spot removal
/// expands the single source request just far enough to retain the full repair
/// ring for every dab that can affect the requested pixels.
pub fn render_image_region_at_source_resolution<E, S>(
    source_width: u32,
    source_height: u32,
    adjustments: Adjustments,
    region: PixelRegion,
    read_source_region: S,
) -> Result<RgbaImage, RegionRenderError<E>>
where
    S: FnOnce(PixelRegion) -> Result<RgbaImage, E>,
{
    render_image_region_at_source_resolution_bounded(
        source_width,
        source_height,
        adjustments,
        region,
        u64::MAX,
        read_source_region,
    )
    .map(|(image, _)| image)
}

/// Bounded form of [`render_image_region_at_source_resolution`].
///
/// `max_staging_pixels` covers every adjusted intermediate, including spot
/// repair rings and filter halos. The source callback can enforce a separate
/// byte or pixel limit for its decoder/provider representation. The returned
/// count is the largest adjusted staging region used by this operation.
pub fn render_image_region_at_source_resolution_bounded<E, S>(
    source_width: u32,
    source_height: u32,
    adjustments: Adjustments,
    region: PixelRegion,
    max_staging_pixels: u64,
    read_source_region: S,
) -> Result<(RgbaImage, u64), RegionRenderError<E>>
where
    S: FnOnce(PixelRegion) -> Result<RgbaImage, E>,
{
    if source_width == 0 || source_height == 0 {
        return Err(RegionRenderError::InvalidSourceDimensions);
    }
    let adjustments = adjustments.sanitized();
    let geometry = AdjustedGeometry::new(source_width, source_height, &adjustments);
    validate_region(region, geometry.output_width, geometry.output_height)?;

    let sharpen_region = expand_region(
        region,
        u32::from(adjustments.sharpening > 0.0) * 2,
        geometry.output_width,
        geometry.output_height,
    );
    let spot_region = expand_region_for_spots(
        sharpen_region,
        &adjustments.spots,
        geometry.output_width,
        geometry.output_height,
    );
    let noise_region = expand_region(
        spot_region,
        u32::from(adjustments.noise_reduction > 0.0) * 4,
        geometry.output_width,
        geometry.output_height,
    );
    let staging_pixels = noise_region.pixel_count();
    if staging_pixels > max_staging_pixels {
        return Err(RegionRenderError::ExceedsStagingPixelLimit);
    }
    let source_region = geometry.required_source_region(noise_region);
    let source = read_source_region(source_region).map_err(RegionRenderError::Source)?;
    if source.dimensions() != (source_region.width, source_region.height) {
        return Err(RegionRenderError::SourceRegionDimensions {
            requested: source_region,
            actual: source.dimensions(),
        });
    }

    let mut pixels = geometry.materialize(noise_region, source_region, &source);
    drop(source);
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
    pixels = crop_region(&pixels, noise_region, spot_region);
    apply_color_adjustments_region(
        &mut pixels,
        &adjustments,
        spot_region.x,
        spot_region.y,
        geometry.output_width,
        geometry.output_height,
    );
    apply_spot_removals_region(
        &mut pixels,
        &adjustments.spots,
        spot_region,
        sharpen_region,
        geometry.output_width,
        geometry.output_height,
    );
    pixels = crop_region(&pixels, spot_region, sharpen_region);
    if adjustments.sharpening > 0.0 {
        let blurred = DynamicImage::ImageRgba8(pixels.clone())
            .blur(1.1)
            .to_rgba8();
        apply_unsharp(&mut pixels, &blurred, adjustments.sharpening / 100.0 * 1.8);
    }
    Ok((crop_region(&pixels, sharpen_region, region), staging_pixels))
}

pub(super) fn apply_spot_removals(image: &mut RgbaImage, spots: &[SpotRemoval]) {
    if image.width() == 0 || image.height() == 0 {
        return;
    }
    let region = PixelRegion {
        x: 0,
        y: 0,
        width: image.width(),
        height: image.height(),
    };
    apply_spot_removals_region(image, spots, region, region, region.width, region.height);
}

fn apply_spot_removals_region(
    image: &mut RgbaImage,
    spots: &[SpotRemoval],
    image_region: PixelRegion,
    affected_region: PixelRegion,
    full_width: u32,
    full_height: u32,
) {
    if spots.is_empty()
        || !spots.iter().any(|spot| {
            SpotGeometry::new(*spot, full_width, full_height).repair_intersects(affected_region)
        })
    {
        return;
    }
    let source = image.clone();
    for spot in spots {
        let geometry = SpotGeometry::new(*spot, full_width, full_height);
        if !geometry.repair_intersects(affected_region) {
            continue;
        }
        let mut total = [0_u64; 3];
        let mut count = 0_u64;
        for y in geometry.outer_top..=geometry.outer_bottom {
            for x in geometry.outer_left..=geometry.outer_right {
                let distance = squared_distance(x, y, geometry.center_x, geometry.center_y) as f32;
                if distance >= geometry.inner_sq && distance <= geometry.outer_sq {
                    let pixel = source.get_pixel(x - image_region.x, y - image_region.y);
                    for channel in 0..3 {
                        total[channel] += u64::from(pixel[channel]);
                    }
                    count += 1;
                }
            }
        }
        if count == 0 {
            continue;
        }
        let repair = [
            total[0] as f32 / count as f32,
            total[1] as f32 / count as f32,
            total[2] as f32 / count as f32,
        ];
        let target = geometry.repair_region().intersection(image_region);
        for y in target.y..target.y + target.height {
            for x in target.x..target.x + target.width {
                let distance =
                    (squared_distance(x, y, geometry.center_x, geometry.center_y) as f32).sqrt();
                if distance > geometry.radius as f32 {
                    continue;
                }
                let feather = ((1.0 - distance / geometry.radius as f32) * 1.8).clamp(0.0, 1.0)
                    * spot.opacity;
                let pixel = image.get_pixel_mut(x - image_region.x, y - image_region.y);
                for channel in 0..3 {
                    pixel[channel] = (pixel[channel] as f32 * (1.0 - feather)
                        + repair[channel] * feather
                        + 0.5) as u8;
                }
            }
        }
    }
}

/// Source-resolution dimensions after development rotation, straighten, and crop.
///
/// Returns `None` when either source axis is zero, matching the region
/// renderer's source validation.
pub fn adjusted_image_dimensions(
    source_width: u32,
    source_height: u32,
    adjustments: &Adjustments,
) -> Option<(u32, u32)> {
    if source_width == 0 || source_height == 0 {
        return None;
    }
    let adjustments = adjustments.clone().sanitized();
    let geometry = AdjustedGeometry::new(source_width, source_height, &adjustments);
    Some((geometry.output_width, geometry.output_height))
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
                    let x =
                        ((crop.x * oriented_width as f32).round() as u32).min(oriented_width - 1);
                    let y =
                        ((crop.y * oriented_height as f32).round() as u32).min(oriented_height - 1);
                    let width = (crop.width * oriented_width as f32).round().max(1.0) as u32;
                    let height = (crop.height * oriented_height as f32).round().max(1.0) as u32;
                    (
                        x,
                        y,
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

fn expand_region_for_spots(
    region: PixelRegion,
    spots: &[SpotRemoval],
    width: u32,
    height: u32,
) -> PixelRegion {
    let mut expanded = region;
    for spot in spots {
        let geometry = SpotGeometry::new(*spot, width, height);
        if geometry.repair_intersects(region) {
            expanded = expanded.union(geometry.outer_region());
        }
    }
    expanded
}

#[derive(Clone, Copy)]
struct SpotGeometry {
    center_x: u32,
    center_y: u32,
    radius: u32,
    outer_left: u32,
    outer_top: u32,
    outer_right: u32,
    outer_bottom: u32,
    inner_sq: f32,
    outer_sq: f32,
}

impl SpotGeometry {
    fn new(spot: SpotRemoval, width: u32, height: u32) -> Self {
        let center_x = (spot.x * width.saturating_sub(1) as f32).round() as u32;
        let center_y = (spot.y * height.saturating_sub(1) as f32).round() as u32;
        let radius = (spot.radius * width.min(height).max(1) as f32)
            .round()
            .max(1.0) as u32;
        let outer = (radius as f32 * 1.9).ceil() as u32;
        Self {
            center_x,
            center_y,
            radius,
            outer_left: center_x.saturating_sub(outer),
            outer_top: center_y.saturating_sub(outer),
            outer_right: center_x.saturating_add(outer).min(width - 1),
            outer_bottom: center_y.saturating_add(outer).min(height - 1),
            inner_sq: (radius as f32 * 1.2).powi(2),
            outer_sq: (outer as f32).powi(2),
        }
    }

    fn repair_region(self) -> PixelRegion {
        PixelRegion {
            x: self.center_x.saturating_sub(self.radius),
            y: self.center_y.saturating_sub(self.radius),
            width: self
                .center_x
                .saturating_add(self.radius)
                .min(self.outer_right)
                - self.center_x.saturating_sub(self.radius)
                + 1,
            height: self
                .center_y
                .saturating_add(self.radius)
                .min(self.outer_bottom)
                - self.center_y.saturating_sub(self.radius)
                + 1,
        }
    }

    fn outer_region(self) -> PixelRegion {
        PixelRegion {
            x: self.outer_left,
            y: self.outer_top,
            width: self.outer_right - self.outer_left + 1,
            height: self.outer_bottom - self.outer_top + 1,
        }
    }

    fn repair_intersects(self, region: PixelRegion) -> bool {
        self.repair_region().intersects(region)
    }
}

impl PixelRegion {
    fn pixel_count(self) -> u64 {
        u64::from(self.width) * u64::from(self.height)
    }

    fn intersects(self, other: Self) -> bool {
        self.x < other.x + other.width
            && other.x < self.x + self.width
            && self.y < other.y + other.height
            && other.y < self.y + self.height
    }

    fn intersection(self, other: Self) -> Self {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let right = (self.x + self.width).min(other.x + other.width);
        let bottom = (self.y + self.height).min(other.y + other.height);
        Self {
            x,
            y,
            width: right.saturating_sub(x),
            height: bottom.saturating_sub(y),
        }
    }

    fn union(self, other: Self) -> Self {
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        let right = (self.x + self.width).max(other.x + other.width);
        let bottom = (self.y + self.height).max(other.y + other.height);
        Self {
            x,
            y,
            width: right - x,
            height: bottom - y,
        }
    }
}

fn squared_distance(x: u32, y: u32, center_x: u32, center_y: u32) -> u64 {
    let dx = i64::from(x) - i64::from(center_x);
    let dy = i64::from(y) - i64::from(center_y);
    (dx * dx + dy * dy) as u64
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
