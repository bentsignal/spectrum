use image::{DynamicImage, Rgba, RgbaImage, imageops::FilterType};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::{Adjustments, ColorGrading, HslAdjustments, ToneCurve};

mod region;
use region::apply_spot_removals;
pub use region::{
    AdjustedPixelSourceMapper, PixelRegion, RegionRenderError, adjusted_image_dimensions,
    render_image_region_at_source_resolution, render_image_region_at_source_resolution_bounded,
};

#[cfg(test)]
mod region_tests;

#[derive(Clone, Copy, Debug, Default)]
pub struct RenderOptions {
    pub max_size: Option<u32>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExportFormat {
    #[default]
    Jpeg,
    Png,
    Tiff,
    Webp,
}

impl ExportFormat {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Jpeg => "jpg",
            Self::Png => "png",
            Self::Tiff => "tiff",
            Self::Webp => "webp",
        }
    }

    pub fn estimate_bytes(self, pixels: u64, quality: u8) -> u64 {
        let bytes_per_pixel = match self {
            Self::Jpeg => 0.18 + quality.clamp(1, 100) as f64 / 100.0 * 0.95,
            Self::Png => 2.1,
            Self::Tiff => 3.2,
            Self::Webp => 1.35,
        };
        (pixels as f64 * bytes_per_pixel) as u64
    }
}

pub fn render_image(
    mut image: DynamicImage,
    adjustments: Adjustments,
    options: RenderOptions,
) -> DynamicImage {
    let adjustments = adjustments.sanitized();
    image = match adjustments.rotation {
        90 => image.rotate90(),
        180 => image.rotate180(),
        270 => image.rotate270(),
        _ => image,
    };
    if adjustments.flip_horizontal {
        image = image.fliph();
    }
    if adjustments.flip_vertical {
        image = image.flipv();
    }
    if adjustments.straighten.abs() > 0.01 {
        image = rotate_filled(&image, adjustments.straighten);
    }
    if let Some(crop) = adjustments.crop {
        let width = image.width();
        let height = image.height();
        let x = ((crop.x * width as f32).round() as u32).min(width - 1);
        let y = ((crop.y * height as f32).round() as u32).min(height - 1);
        let w = (crop.width * width as f32).round().max(1.0) as u32;
        let h = (crop.height * height as f32).round().max(1.0) as u32;
        image = image.crop_imm(x, y, w.min(width - x), h.min(height - y));
    }
    if let Some(max_size) = options
        .max_size
        .filter(|size| *size > 0 && (image.width() > *size || image.height() > *size))
    {
        image = image.resize(max_size, max_size, FilterType::Triangle);
    }
    if !has_pixel_adjustments(&adjustments) {
        return image;
    }
    let mut pixels = image.to_rgba8();
    if adjustments.noise_reduction > 0.0 {
        pixels = blend_images(
            &pixels,
            &DynamicImage::ImageRgba8(pixels.clone())
                .blur(1.6)
                .to_rgba8(),
            adjustments.noise_reduction / 100.0 * 0.75,
        );
    }
    apply_color_adjustments(&mut pixels, &adjustments);
    if !adjustments.spots.is_empty() {
        apply_spot_removals(&mut pixels, &adjustments.spots);
    }
    if adjustments.sharpening > 0.0 {
        let blurred = DynamicImage::ImageRgba8(pixels.clone())
            .blur(1.1)
            .to_rgba8();
        apply_unsharp(&mut pixels, &blurred, adjustments.sharpening / 100.0 * 1.8);
    }
    DynamicImage::ImageRgba8(pixels)
}

fn rotate_filled(image: &DynamicImage, degrees: f32) -> DynamicImage {
    let source = image.to_rgba8();
    let (width, height) = source.dimensions();
    let mut output = RgbaImage::new(width, height);
    let radians = degrees.to_radians();
    let (sin, cos) = radians.sin_cos();
    let aspect = width as f32 / height.max(1) as f32;
    let zoom = (cos.abs() + aspect * sin.abs())
        .max(cos.abs() + sin.abs() / aspect)
        .max(1.0);
    let cx = (width as f32 - 1.0) * 0.5;
    let cy = (height as f32 - 1.0) * 0.5;
    for (x, y, pixel) in output.enumerate_pixels_mut() {
        let dx = (x as f32 - cx) / zoom;
        let dy = (y as f32 - cy) / zoom;
        let sx = cos * dx + sin * dy + cx;
        let sy = -sin * dx + cos * dy + cy;
        *pixel = sample_bilinear(&source, sx, sy);
    }
    DynamicImage::ImageRgba8(output)
}

fn sample_bilinear(image: &RgbaImage, x: f32, y: f32) -> Rgba<u8> {
    let x = x.clamp(0.0, image.width().saturating_sub(1) as f32);
    let y = y.clamp(0.0, image.height().saturating_sub(1) as f32);
    let x0 = x.floor() as u32;
    let y0 = y.floor() as u32;
    let x1 = (x0 + 1).min(image.width() - 1);
    let y1 = (y0 + 1).min(image.height() - 1);
    let tx = x - x0 as f32;
    let ty = y - y0 as f32;
    let mut out = [0; 4];
    for (channel, value) in out.iter_mut().enumerate() {
        let top = image.get_pixel(x0, y0)[channel] as f32 * (1.0 - tx)
            + image.get_pixel(x1, y0)[channel] as f32 * tx;
        let bottom = image.get_pixel(x0, y1)[channel] as f32 * (1.0 - tx)
            + image.get_pixel(x1, y1)[channel] as f32 * tx;
        *value = (top * (1.0 - ty) + bottom * ty + 0.5) as u8;
    }
    Rgba(out)
}

fn blend_images(source: &RgbaImage, blurred: &RgbaImage, amount: f32) -> RgbaImage {
    let mut output = source.clone();
    for (pixel, blur) in output.pixels_mut().zip(blurred.pixels()) {
        for channel in 0..3 {
            pixel[channel] = (pixel[channel] as f32 * (1.0 - amount)
                + blur[channel] as f32 * amount
                + 0.5) as u8;
        }
    }
    output
}

fn apply_unsharp(image: &mut RgbaImage, blurred: &RgbaImage, amount: f32) {
    for (pixel, blur) in image.pixels_mut().zip(blurred.pixels()) {
        for channel in 0..3 {
            let value =
                pixel[channel] as f32 + (pixel[channel] as f32 - blur[channel] as f32) * amount;
            pixel[channel] = value.clamp(0.0, 255.0) as u8;
        }
    }
}

fn apply_color_adjustments(image: &mut RgbaImage, a: &Adjustments) {
    let (width, height) = image.dimensions();
    apply_color_adjustments_region(image, a, 0, 0, width, height);
}

fn apply_color_adjustments_region(
    image: &mut RgbaImage,
    a: &Adjustments,
    origin_x: u32,
    origin_y: u32,
    full_width: u32,
    full_height: u32,
) {
    if !has_color_adjustments(a) {
        return;
    }
    let width_pixels = image.width().max(1) as usize;
    let width = full_width.max(1) as f32;
    let height = full_height.max(1) as f32;
    let exposure = 2.0_f32.powf(a.exposure);
    let temperature = a.temperature / 100.0;
    let tint = a.tint / 100.0;
    let contrast = a.contrast / 100.0;
    let highlights = a.highlights / 100.0;
    let shadows = a.shadows / 100.0;
    let whites = a.whites / 100.0;
    let blacks = a.blacks / 100.0;
    let texture = a.texture / 100.0;
    let clarity = a.clarity / 100.0;
    let dehaze = a.dehaze / 100.0;
    let saturation = a.saturation / 100.0;
    let vibrance = a.vibrance / 100.0;
    let vignette = a.vignette / 100.0;
    let grading_active = !a.color_grading.is_identity();
    let hsl_active = !a.hsl.is_identity();
    let master_curve = (!a.curves.master.is_identity()).then_some(&a.curves.master);
    let red_curve = (!a.curves.red.is_identity()).then_some(&a.curves.red);
    let green_curve = (!a.curves.green.is_identity()).then_some(&a.curves.green);
    let blue_curve = (!a.curves.blue.is_identity()).then_some(&a.curves.blue);
    image
        .as_mut()
        .par_chunks_mut(4)
        .enumerate()
        .for_each(|(index, pixel)| {
            let x = origin_x as usize + index % width_pixels;
            let y = origin_y as usize + index / width_pixels;
            let alpha = pixel[3];
            let mut rgb = [
                pixel[0] as f32 / 255.0,
                pixel[1] as f32 / 255.0,
                pixel[2] as f32 / 255.0,
            ];
            rgb[0] *= (1.0 + temperature * 0.28 + tint * 0.08) * exposure;
            rgb[1] *= (1.0 - tint * 0.16) * exposure;
            rgb[2] *= (1.0 - temperature * 0.28 + tint * 0.08) * exposure;
            let mut luma = luminance(rgb);
            let tonal = shadows * 0.32 * (1.0 - luma).powi(2)
                + highlights * 0.32 * luma.powi(2)
                + blacks * 0.16 * (1.0 - luma)
                + whites * 0.16 * luma;
            for channel in &mut rgb {
                *channel += tonal;
            }
            let contrast_factor = if contrast >= 0.0 {
                1.0 + contrast * 1.5
            } else {
                1.0 + contrast * 0.85
            };
            luma = luminance(rgb).clamp(0.0, 1.0);
            let midtones = 4.0 * luma * (1.0 - luma);
            for channel in &mut rgb {
                *channel = (*channel - 0.5) * contrast_factor + 0.5;
                *channel += (*channel - luma) * texture * 0.18;
                *channel += (*channel - 0.5) * clarity * 0.22 * midtones;
                *channel = (*channel - 0.5) * (1.0 + dehaze * 0.55) + 0.5 - dehaze * 0.025;
            }
            if hsl_active {
                let (mut hue, mut sat, mut light) = rgb_to_hsl(rgb);
                apply_hsl_mixer(&mut hue, &mut sat, &mut light, &a.hsl);
                rgb = hsl_to_rgb(hue, sat, light);
            }
            luma = luminance(rgb);
            let current_saturation = rgb.iter().copied().fold(f32::MIN, f32::max)
                - rgb.iter().copied().fold(f32::MAX, f32::min);
            let saturation_factor =
                (1.0 + saturation) * (1.0 + vibrance * (1.0 - current_saturation.clamp(0.0, 1.0)));
            for channel in &mut rgb {
                *channel = luma + (*channel - luma) * saturation_factor;
            }
            if grading_active {
                apply_color_grading(&mut rgb, &a.color_grading);
            }
            apply_curves(&mut rgb, master_curve, [red_curve, green_curve, blue_curve]);
            if vignette != 0.0 {
                let nx = (x as f32 + 0.5) / width * 2.0 - 1.0;
                let ny = (y as f32 + 0.5) / height * 2.0 - 1.0;
                let multiplier = (1.0
                    + vignette * ((nx * nx + ny * ny) / 2.0).clamp(0.0, 1.0).powf(1.4) * 0.72)
                    .max(0.0);
                for channel in &mut rgb {
                    *channel *= multiplier;
                }
            }
            for (index, channel) in rgb.into_iter().enumerate() {
                pixel[index] = (channel.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            }
            pixel[3] = alpha;
        });
}

fn has_pixel_adjustments(a: &Adjustments) -> bool {
    a.noise_reduction > 0.0 || a.sharpening > 0.0 || !a.spots.is_empty() || has_color_adjustments(a)
}

fn has_color_adjustments(a: &Adjustments) -> bool {
    a.exposure != 0.0
        || a.temperature != 0.0
        || a.tint != 0.0
        || a.contrast != 0.0
        || a.highlights != 0.0
        || a.shadows != 0.0
        || a.whites != 0.0
        || a.blacks != 0.0
        || a.texture != 0.0
        || a.clarity != 0.0
        || a.dehaze != 0.0
        || a.saturation != 0.0
        || a.vibrance != 0.0
        || a.vignette != 0.0
        || !a.hsl.is_identity()
        || !a.curves.is_identity()
        || !a.color_grading.is_identity()
}

fn apply_color_grading(rgb: &mut [f32; 3], grading: &ColorGrading) {
    let luma = luminance(*rgb).clamp(0.0, 1.0);
    let balance = grading.balance / 100.0 * 0.3;
    let shadow_weight = ((1.0 - luma + balance).clamp(0.0, 1.0)).powi(2);
    let highlight_weight = ((luma - balance).clamp(0.0, 1.0)).powi(2);
    let midtone_weight = (4.0 * luma * (1.0 - luma)).clamp(0.0, 1.0);
    for (grade, weight) in [
        (&grading.shadows, shadow_weight),
        (&grading.midtones, midtone_weight),
        (&grading.highlights, highlight_weight),
    ] {
        let strength = grade.saturation / 100.0 * weight * 0.42;
        if strength > 0.0 {
            let tint = hsl_to_rgb(grade.hue / 360.0, 1.0, 0.5);
            for channel in 0..3 {
                rgb[channel] += (tint[channel] - 0.5) * strength;
            }
        }
        let lift = grade.luminance / 100.0 * weight * 0.24;
        for channel in rgb.iter_mut() {
            *channel += lift;
        }
    }
}

fn apply_hsl_mixer(hue: &mut f32, saturation: &mut f32, lightness: &mut f32, hsl: &HslAdjustments) {
    const CENTERS: [f32; 8] = [
        0.0,
        30.0 / 360.0,
        60.0 / 360.0,
        120.0 / 360.0,
        180.0 / 360.0,
        240.0 / 360.0,
        285.0 / 360.0,
        330.0 / 360.0,
    ];
    let mut hue_shift = 0.0;
    let mut sat_shift = 0.0;
    let mut light_shift = 0.0;
    let mut total = 0.0;
    for (index, center) in CENTERS.into_iter().enumerate() {
        let distance = ((*hue - center).abs()).min(1.0 - (*hue - center).abs());
        let weight = (1.0 - distance / 0.125).clamp(0.0, 1.0);
        let band = hsl.band(index);
        total += weight;
        hue_shift += band.hue / 100.0 * (15.0 / 360.0) * weight;
        sat_shift += band.saturation / 100.0 * weight;
        light_shift += band.luminance / 100.0 * weight;
    }
    if total > 0.0 {
        *hue = (*hue + hue_shift / total).rem_euclid(1.0);
        *saturation = (*saturation * (1.0 + sat_shift / total)).clamp(0.0, 1.0);
        *lightness = (*lightness + light_shift / total * 0.35).clamp(0.0, 1.0);
    }
}

fn apply_curves(rgb: &mut [f32; 3], master: Option<&ToneCurve>, channels: [Option<&ToneCurve>; 3]) {
    if let Some(master) = master {
        for channel in rgb.iter_mut() {
            *channel = master.evaluate(channel.clamp(0.0, 1.0));
        }
    }
    for (channel, curve) in rgb.iter_mut().zip(channels) {
        if let Some(curve) = curve {
            *channel = curve.evaluate(channel.clamp(0.0, 1.0));
        }
    }
}

fn rgb_to_hsl(rgb: [f32; 3]) -> (f32, f32, f32) {
    let max = rgb.into_iter().fold(f32::MIN, f32::max);
    let min = rgb.into_iter().fold(f32::MAX, f32::min);
    let light = (max + min) * 0.5;
    let delta = max - min;
    if delta < f32::EPSILON {
        return (0.0, 0.0, light.clamp(0.0, 1.0));
    }
    let sat = delta / (1.0 - (2.0 * light - 1.0).abs()).max(f32::EPSILON);
    let hue = if max == rgb[0] {
        ((rgb[1] - rgb[2]) / delta).rem_euclid(6.0)
    } else if max == rgb[1] {
        (rgb[2] - rgb[0]) / delta + 2.0
    } else {
        (rgb[0] - rgb[1]) / delta + 4.0
    } / 6.0;
    (hue, sat.clamp(0.0, 1.0), light.clamp(0.0, 1.0))
}

fn hsl_to_rgb(hue: f32, saturation: f32, lightness: f32) -> [f32; 3] {
    let c = (1.0 - (2.0 * lightness - 1.0).abs()) * saturation;
    let h = hue * 6.0;
    let x = c * (1.0 - (h.rem_euclid(2.0) - 1.0).abs());
    let base = match h as i32 {
        0 => [c, x, 0.0],
        1 => [x, c, 0.0],
        2 => [0.0, c, x],
        3 => [0.0, x, c],
        4 => [x, 0.0, c],
        _ => [c, 0.0, x],
    };
    let m = lightness - c * 0.5;
    [base[0] + m, base[1] + m, base[2] + m]
}

fn luminance(rgb: [f32; 3]) -> f32 {
    rgb[0] * 0.2126 + rgb[1] * 0.7152 + rgb[2] * 0.0722
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ColorGrade, CropRect, CurvePoint, SpotRemoval, ToneCurve, ToneCurves};
    use image::{GenericImageView, Rgba, RgbaImage};
    use std::time::Instant;

    #[test]
    fn exposure_brightens_pixels() {
        let source = DynamicImage::ImageRgba8(RgbaImage::from_pixel(2, 2, Rgba([32, 32, 32, 255])));
        let rendered = render_image(
            source,
            Adjustments {
                exposure: 1.0,
                ..Default::default()
            },
            RenderOptions::default(),
        );
        assert!(rendered.to_rgba8().get_pixel(0, 0)[0] > 32);
    }

    #[test]
    fn identity_render_preserves_pixels() {
        let source = RgbaImage::from_fn(16, 12, |x, y| {
            Rgba([x as u8 * 11, y as u8 * 13, (x + y) as u8 * 7, 255])
        });
        let rendered = render_image(
            DynamicImage::ImageRgba8(source.clone()),
            Adjustments::default(),
            RenderOptions::default(),
        );
        assert_eq!(rendered.to_rgba8(), source);
    }

    #[test]
    fn hsl_mixer_still_changes_target_color() {
        let source =
            DynamicImage::ImageRgba8(RgbaImage::from_pixel(8, 8, Rgba([20, 90, 220, 255])));
        let mut adjustments = Adjustments::default();
        adjustments.hsl.blue.saturation = -80.0;
        let rendered = render_image(source, adjustments, RenderOptions::default()).to_rgba8();
        assert!(rendered.get_pixel(0, 0)[2] < 220);
    }

    #[test]
    fn color_grading_tints_midtones() {
        let source =
            DynamicImage::ImageRgba8(RgbaImage::from_pixel(8, 8, Rgba([110, 110, 110, 255])));
        let mut adjustments = Adjustments::default();
        adjustments.color_grading.midtones = ColorGrade {
            hue: 30.0,
            saturation: 80.0,
            luminance: 0.0,
        };
        let rendered = render_image(source, adjustments, RenderOptions::default()).to_rgba8();
        let pixel = rendered.get_pixel(0, 0);
        assert!(pixel[0] > pixel[2]);
    }

    #[test]
    fn spot_removal_repairs_isolated_dust_pixel() {
        let mut source = RgbaImage::from_pixel(21, 21, Rgba([30, 30, 30, 255]));
        source.put_pixel(10, 10, Rgba([245, 245, 245, 255]));
        let rendered = render_image(
            DynamicImage::ImageRgba8(source),
            Adjustments {
                spots: vec![SpotRemoval {
                    x: 0.5,
                    y: 0.5,
                    radius: 0.12,
                    opacity: 1.0,
                }],
                ..Default::default()
            },
            RenderOptions::default(),
        )
        .to_rgba8();
        assert!(rendered.get_pixel(10, 10)[0] < 80);
    }

    #[test]
    fn crop_and_rotation_change_dimensions() {
        let source = DynamicImage::new_rgba8(4, 2);
        let rendered = render_image(
            source,
            Adjustments {
                rotation: 90,
                crop: Some(CropRect {
                    x: 0.0,
                    y: 0.0,
                    width: 0.5,
                    height: 1.0,
                }),
                ..Default::default()
            },
            RenderOptions::default(),
        );
        assert_eq!(rendered.dimensions(), (1, 4));
    }

    #[test]
    #[ignore = "manual release-mode performance benchmark"]
    fn interactive_preview_benchmark() {
        let source = DynamicImage::ImageRgba8(RgbaImage::from_fn(1800, 1200, |x, y| {
            Rgba([(x % 256) as u8, (y % 256) as u8, ((x + y) % 256) as u8, 255])
        }));
        let adjustments = Adjustments {
            exposure: 0.35,
            contrast: 12.0,
            shadows: 18.0,
            vibrance: 8.0,
            curves: ToneCurves {
                master: ToneCurve {
                    points: vec![
                        CurvePoint { x: 0.0, y: 0.0 },
                        CurvePoint { x: 0.4, y: 0.35 },
                        CurvePoint { x: 1.0, y: 1.0 },
                    ],
                },
                ..Default::default()
            },
            ..Default::default()
        };
        let iterations = 4;
        let started = Instant::now();
        for _ in 0..iterations {
            std::hint::black_box(render_image(
                source.clone(),
                adjustments.clone(),
                RenderOptions::default(),
            ));
        }
        let elapsed = started.elapsed();
        eprintln!(
            "interactive preview: {:.1} ms/frame",
            elapsed.as_secs_f64() * 1000.0 / iterations as f64
        );
    }

    #[test]
    fn red_curve_changes_red_only() {
        let source =
            DynamicImage::ImageRgba8(RgbaImage::from_pixel(1, 1, Rgba([128, 128, 128, 255])));
        let mut a = Adjustments::default();
        a.curves.red = ToneCurve {
            points: vec![CurvePoint { x: 0.0, y: 0.0 }, CurvePoint { x: 1.0, y: 0.5 }],
        };
        let pixel = render_image(source, a, RenderOptions::default())
            .to_rgba8()
            .get_pixel(0, 0)
            .0;
        assert!(pixel[0] < pixel[1]);
        assert_eq!(pixel[1], pixel[2]);
    }
}
