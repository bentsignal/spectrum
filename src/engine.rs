use std::{fs::File, io::BufWriter, path::Path};

use anyhow::{Context, Result, bail};
use image::{DynamicImage, ImageEncoder, Rgba, RgbaImage, imageops::FilterType};
use rawler::{Orientation, imgop::develop::RawDevelop};

use crate::{Adjustments, HslAdjustments, Photo, ToneCurves, project::is_raw_image};

#[derive(Clone, Copy, Debug, Default)]
pub struct RenderOptions {
    pub max_size: Option<u32>,
}

pub fn render_photo(photo: &Photo, options: RenderOptions) -> Result<DynamicImage> {
    let decoded = decode_photo(photo, options.max_size)?;
    Ok(render_image(
        decoded,
        photo.adjustments.clone(),
        RenderOptions::default(),
    ))
}

/// Decode and optionally downsample a source. ARW development stays entirely in Rust.
pub fn decode_photo(photo: &Photo, max_size: Option<u32>) -> Result<DynamicImage> {
    let mut decoded = if is_raw_image(&photo.path) {
        let raw = rawler::decode_file(&photo.path)
            .with_context(|| format!("could not decode Sony RAW {}", photo.path.display()))?;
        let orientation = raw.orientation;
        let developed = RawDevelop::default()
            .develop_intermediate(&raw)
            .with_context(|| format!("could not develop Sony RAW {}", photo.path.display()))?
            .to_dynamic_image()
            .with_context(|| format!("Sony RAW produced no image: {}", photo.path.display()))?;
        apply_orientation(developed, orientation)
    } else {
        image::ImageReader::open(&photo.path)
            .with_context(|| format!("could not open {}", photo.path.display()))?
            .with_guessed_format()?
            .decode()
            .with_context(|| format!("could not decode {}", photo.path.display()))?
    };
    if let Some(max_size) =
        max_size.filter(|size| *size > 0 && (decoded.width() > *size || decoded.height() > *size))
    {
        decoded = decoded.resize(max_size, max_size, FilterType::Triangle);
    }
    Ok(decoded)
}

fn apply_orientation(mut image: DynamicImage, orientation: Orientation) -> DynamicImage {
    let (transpose, horizontal, vertical) = orientation.to_flips();
    if horizontal {
        image = image.fliph();
    }
    if vertical {
        image = image.flipv();
    }
    if transpose {
        let source = image.to_rgba8();
        let mut output = RgbaImage::new(source.height(), source.width());
        for (x, y, pixel) in source.enumerate_pixels() {
            output.put_pixel(y, x, *pixel);
        }
        image = DynamicImage::ImageRgba8(output);
    }
    image
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
        let x = (crop.x * width as f32).round() as u32;
        let y = (crop.y * height as f32).round() as u32;
        let w = (crop.width * width as f32).round().max(1.0) as u32;
        let h = (crop.height * height as f32).round().max(1.0) as u32;
        image = image.crop_imm(
            x.min(width - 1),
            y.min(height - 1),
            w.min(width - x),
            h.min(height - y),
        );
    }
    if let Some(max_size) = options
        .max_size
        .filter(|size| *size > 0 && (image.width() > *size || image.height() > *size))
    {
        image = image.resize(max_size, max_size, FilterType::Triangle);
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
    if adjustments.sharpening > 0.0 {
        let blurred = DynamicImage::ImageRgba8(pixels.clone())
            .blur(1.1)
            .to_rgba8();
        apply_unsharp(&mut pixels, &blurred, adjustments.sharpening / 100.0 * 1.8);
    }
    DynamicImage::ImageRgba8(pixels)
}

pub fn export_photo(
    photo: &Photo,
    destination: &Path,
    options: RenderOptions,
    jpeg_quality: u8,
) -> Result<()> {
    let rendered = render_photo(photo, options)?;
    if let Some(parent) = destination
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)?;
    }
    let extension = destination
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if matches!(extension.as_str(), "jpg" | "jpeg") {
        let writer = BufWriter::new(File::create(destination)?);
        let rgb = rendered.to_rgb8();
        image::codecs::jpeg::JpegEncoder::new_with_quality(writer, jpeg_quality.clamp(1, 100))
            .write_image(
                rgb.as_raw(),
                rgb.width(),
                rgb.height(),
                image::ExtendedColorType::Rgb8,
            )?;
    } else if matches!(extension.as_str(), "png" | "tif" | "tiff" | "webp") {
        rendered.save(destination)?;
    } else {
        bail!("export path must end in jpg, jpeg, png, tif, tiff, or webp");
    }
    Ok(())
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
    let width = image.width().max(1) as f32;
    let height = image.height().max(1) as f32;
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
    for (x, y, pixel) in image.enumerate_pixels_mut() {
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
        let (mut hue, mut sat, mut light) = rgb_to_hsl(rgb);
        apply_hsl_mixer(&mut hue, &mut sat, &mut light, &a.hsl);
        rgb = hsl_to_rgb(hue, sat, light);
        luma = luminance(rgb);
        let current_saturation = rgb.iter().copied().fold(f32::MIN, f32::max)
            - rgb.iter().copied().fold(f32::MAX, f32::min);
        let saturation_factor =
            (1.0 + saturation) * (1.0 + vibrance * (1.0 - current_saturation.clamp(0.0, 1.0)));
        for channel in &mut rgb {
            *channel = luma + (*channel - luma) * saturation_factor;
        }
        apply_curves(&mut rgb, &a.curves);
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

fn apply_curves(rgb: &mut [f32; 3], curves: &ToneCurves) {
    for channel in rgb.iter_mut() {
        *channel = curves.master.evaluate(channel.clamp(0.0, 1.0));
    }
    rgb[0] = curves.red.evaluate(rgb[0]);
    rgb[1] = curves.green.evaluate(rgb[1]);
    rgb[2] = curves.blue.evaluate(rgb[2]);
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
    use crate::{CropRect, CurvePoint, ToneCurve};
    use image::{GenericImageView, Rgba, RgbaImage};

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

    #[test]
    fn raw_orientation_swaps_dimensions_when_transposed() {
        let source = DynamicImage::new_rgba8(4, 2);
        assert_eq!(
            apply_orientation(source, Orientation::Rotate90).dimensions(),
            (2, 4)
        );
    }
}
