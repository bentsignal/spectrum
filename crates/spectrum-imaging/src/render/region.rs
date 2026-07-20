use image::{DynamicImage, Rgba, RgbaImage};

use super::{apply_color_adjustments_region, apply_unsharp, blend_images};
use crate::Adjustments;

/// A pixel-space crop in an adjusted image.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PixelRegion {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// Renders an exact crop of an adjusted image from an on-demand source.
///
/// The returned pixels follow [`render_image`] semantics, including geometry,
/// blur, vignette, spot removal, and sharpening, while intermediate allocation
/// stays proportional to `region` plus the filters' finite halos.
pub fn render_image_region<S>(
    source_width: u32,
    source_height: u32,
    adjustments: Adjustments,
    region: PixelRegion,
    mut source: S,
) -> Result<RgbaImage, String>
where
    S: FnMut(u32, u32) -> Rgba<u8>,
{
    if source_width == 0 || source_height == 0 {
        return Err("adjusted image source must have positive dimensions".into());
    }
    let adjustments = adjustments.sanitized();
    let geometry = AdjustedGeometry::new(source_width, source_height, &adjustments);
    validate_region(region, geometry.output_width, geometry.output_height)?;
    let sharpen_radius = u32::from(adjustments.sharpening > 0.0) * 2;
    let sharpen_region = expand_region(
        region,
        sharpen_radius,
        geometry.output_width,
        geometry.output_height,
    );
    let noise_radius = u32::from(adjustments.noise_reduction > 0.0) * 4;
    let noise_region = expand_region(
        sharpen_region,
        noise_radius,
        geometry.output_width,
        geometry.output_height,
    );
    let mut pixels = geometry.materialize(noise_region, &mut source);
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
    if !adjustments.spots.is_empty() {
        apply_spot_removals_region(
            &mut pixels,
            sharpen_region,
            geometry,
            &adjustments,
            &mut source,
        );
    }
    if adjustments.sharpening > 0.0 {
        let blurred = DynamicImage::ImageRgba8(pixels.clone())
            .blur(1.1)
            .to_rgba8();
        apply_unsharp(&mut pixels, &blurred, adjustments.sharpening / 100.0 * 1.8);
    }
    Ok(crop_region(&pixels, sharpen_region, region))
}

/// Dimensions after development geometry (rotation and crop) is applied.
pub fn adjusted_image_dimensions(
    source_width: u32,
    source_height: u32,
    adjustments: &Adjustments,
) -> (u32, u32) {
    let sanitized = adjustments.clone().sanitized();
    let geometry = AdjustedGeometry::new(source_width.max(1), source_height.max(1), &sanitized);
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

    fn materialize<S>(&self, region: PixelRegion, source: &mut S) -> RgbaImage
    where
        S: FnMut(u32, u32) -> Rgba<u8>,
    {
        RgbaImage::from_fn(region.width, region.height, |x, y| {
            self.pixel(region.x + x, region.y + y, source)
        })
    }

    fn pixel<S>(&self, x: u32, y: u32, source: &mut S) -> Rgba<u8>
    where
        S: FnMut(u32, u32) -> Rgba<u8>,
    {
        let x = x + self.crop_x;
        let y = y + self.crop_y;
        if self.straighten.abs() <= 0.01 {
            return self.oriented_pixel(x, y, source);
        }
        let radians = self.straighten.to_radians();
        let (sin, cos) = radians.sin_cos();
        let aspect = self.oriented_width as f32 / self.oriented_height.max(1) as f32;
        let zoom = (cos.abs() + aspect * sin.abs())
            .max(cos.abs() + sin.abs() / aspect)
            .max(1.0);
        let cx = (self.oriented_width as f32 - 1.0) * 0.5;
        let cy = (self.oriented_height as f32 - 1.0) * 0.5;
        let dx = (x as f32 - cx) / zoom;
        let dy = (y as f32 - cy) / zoom;
        let sample_x =
            (cos * dx + sin * dy + cx).clamp(0.0, self.oriented_width.saturating_sub(1) as f32);
        let sample_y =
            (-sin * dx + cos * dy + cy).clamp(0.0, self.oriented_height.saturating_sub(1) as f32);
        self.oriented_bilinear(sample_x, sample_y, source)
    }

    fn oriented_bilinear<S>(&self, x: f32, y: f32, source: &mut S) -> Rgba<u8>
    where
        S: FnMut(u32, u32) -> Rgba<u8>,
    {
        let x0 = x.floor() as u32;
        let y0 = y.floor() as u32;
        let x1 = (x0 + 1).min(self.oriented_width - 1);
        let y1 = (y0 + 1).min(self.oriented_height - 1);
        let tx = x - x0 as f32;
        let ty = y - y0 as f32;
        let samples = [
            self.oriented_pixel(x0, y0, source),
            self.oriented_pixel(x1, y0, source),
            self.oriented_pixel(x0, y1, source),
            self.oriented_pixel(x1, y1, source),
        ];
        let mut output = [0; 4];
        for channel in 0..4 {
            let top = samples[0][channel] as f32 * (1.0 - tx) + samples[1][channel] as f32 * tx;
            let bottom = samples[2][channel] as f32 * (1.0 - tx) + samples[3][channel] as f32 * tx;
            output[channel] = (top * (1.0 - ty) + bottom * ty + 0.5) as u8;
        }
        Rgba(output)
    }

    fn oriented_pixel<S>(&self, mut x: u32, mut y: u32, source: &mut S) -> Rgba<u8>
    where
        S: FnMut(u32, u32) -> Rgba<u8>,
    {
        if self.flip_vertical {
            y = self.oriented_height - y - 1;
        }
        if self.flip_horizontal {
            x = self.oriented_width - x - 1;
        }
        let (x, y) = match self.rotation {
            90 => (y, self.source_height - x - 1),
            180 => (self.source_width - x - 1, self.source_height - y - 1),
            270 => (self.source_width - y - 1, x),
            _ => (x, y),
        };
        source(x, y)
    }
}

fn validate_region(region: PixelRegion, width: u32, height: u32) -> Result<(), String> {
    if region.width == 0 || region.height == 0 {
        return Err("adjusted image region must have positive dimensions".into());
    }
    let right = region
        .x
        .checked_add(region.width)
        .ok_or_else(|| "adjusted image region overflows horizontally".to_owned())?;
    let bottom = region
        .y
        .checked_add(region.height)
        .ok_or_else(|| "adjusted image region overflows vertically".to_owned())?;
    if right > width || bottom > height {
        return Err("adjusted image region exceeds the adjusted image".into());
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

fn apply_spot_removals_region<S>(
    image: &mut RgbaImage,
    region: PixelRegion,
    geometry: AdjustedGeometry,
    adjustments: &Adjustments,
    source_pixels: &mut S,
) where
    S: FnMut(u32, u32) -> Rgba<u8>,
{
    let source = image.clone();
    let width = geometry.output_width as i32;
    let height = geometry.output_height as i32;
    let scale = width.min(height).max(1) as f32;
    for spot in &adjustments.spots {
        let cx = (spot.x * (width - 1).max(0) as f32).round() as i32;
        let cy = (spot.y * (height - 1).max(0) as f32).round() as i32;
        let radius = (spot.radius * scale).round().max(1.0) as i32;
        if !circle_intersects_region(cx, cy, radius, region) {
            continue;
        }
        let outer = (radius as f32 * 1.9).ceil() as i32;
        let inner_sq = (radius as f32 * 1.2).powi(2);
        let outer_sq = (outer as f32).powi(2);
        let mut total = [0_u64; 3];
        let mut count = 0_u64;
        for y in (cy - outer).max(0)..=(cy + outer).min(height - 1) {
            for x in (cx - outer).max(0)..=(cx + outer).min(width - 1) {
                let distance = ((x - cx).pow(2) + (y - cy).pow(2)) as f32;
                if distance < inner_sq || distance > outer_sq {
                    continue;
                }
                let pixel = if region_contains(region, x as u32, y as u32) {
                    *source.get_pixel(x as u32 - region.x, y as u32 - region.y)
                } else {
                    pre_spot_pixel(geometry, adjustments, x as u32, y as u32, source_pixels)
                };
                for channel in 0..3 {
                    total[channel] += u64::from(pixel[channel]);
                }
                count += 1;
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
        let left = (cx - radius).max(region.x as i32).max(0);
        let top = (cy - radius).max(region.y as i32).max(0);
        let right = (cx + radius)
            .min((region.x + region.width - 1) as i32)
            .min(width - 1);
        let bottom = (cy + radius)
            .min((region.y + region.height - 1) as i32)
            .min(height - 1);
        for y in top..=bottom {
            for x in left..=right {
                let distance = (((x - cx).pow(2) + (y - cy).pow(2)) as f32).sqrt();
                if distance > radius as f32 {
                    continue;
                }
                let feather =
                    ((1.0 - distance / radius as f32) * 1.8).clamp(0.0, 1.0) * spot.opacity;
                let pixel = image.get_pixel_mut(x as u32 - region.x, y as u32 - region.y);
                for channel in 0..3 {
                    pixel[channel] = (pixel[channel] as f32 * (1.0 - feather)
                        + repair[channel] * feather
                        + 0.5) as u8;
                }
            }
        }
    }
}

fn pre_spot_pixel<S>(
    geometry: AdjustedGeometry,
    adjustments: &Adjustments,
    x: u32,
    y: u32,
    source: &mut S,
) -> Rgba<u8>
where
    S: FnMut(u32, u32) -> Rgba<u8>,
{
    let target = PixelRegion {
        x,
        y,
        width: 1,
        height: 1,
    };
    let noise_region = expand_region(
        target,
        u32::from(adjustments.noise_reduction > 0.0) * 4,
        geometry.output_width,
        geometry.output_height,
    );
    let mut pixels = geometry.materialize(noise_region, source);
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
    let mut pixel = crop_region(&pixels, noise_region, target);
    apply_color_adjustments_region(
        &mut pixel,
        adjustments,
        x,
        y,
        geometry.output_width,
        geometry.output_height,
    );
    *pixel.get_pixel(0, 0)
}

fn circle_intersects_region(cx: i32, cy: i32, radius: i32, region: PixelRegion) -> bool {
    let nearest_x = cx.clamp(region.x as i32, (region.x + region.width - 1) as i32);
    let nearest_y = cy.clamp(region.y as i32, (region.y + region.height - 1) as i32);
    (nearest_x - cx).pow(2) + (nearest_y - cy).pow(2) <= radius.pow(2)
}

fn region_contains(region: PixelRegion, x: u32, y: u32) -> bool {
    x >= region.x && x < region.x + region.width && y >= region.y && y < region.y + region.height
}
