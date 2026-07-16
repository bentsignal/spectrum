use std::{fs::File, io::BufWriter, path::Path};

use anyhow::{Context, Result, bail};
use image::{DynamicImage, ImageEncoder, RgbaImage, imageops::FilterType};

use crate::{Adjustments, Photo};

#[derive(Clone, Copy, Debug, Default)]
pub struct RenderOptions {
    /// Long-edge pixel limit. `None` renders at the source size.
    pub max_size: Option<u32>,
}

pub fn render_photo(photo: &Photo, options: RenderOptions) -> Result<DynamicImage> {
    let decoded = decode_photo(photo, options.max_size)?;
    Ok(render_image(
        decoded,
        photo.adjustments,
        RenderOptions::default(),
    ))
}

/// Decode and optionally downsample a source without applying edits. The GUI
/// caches this result so interactive slider changes never decode the file again.
pub fn decode_photo(photo: &Photo, max_size: Option<u32>) -> Result<DynamicImage> {
    let mut decoded = image::ImageReader::open(&photo.path)
        .with_context(|| format!("could not open {}", photo.path.display()))?
        .with_guessed_format()?
        .decode()
        .with_context(|| format!("could not decode {}", photo.path.display()))?;
    if let Some(max_size) =
        max_size.filter(|size| *size > 0 && (decoded.width() > *size || decoded.height() > *size))
    {
        decoded = decoded.resize(max_size, max_size, FilterType::Triangle);
    }
    Ok(decoded)
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
    if let Some(max_size) = options
        .max_size
        .filter(|size| *size > 0 && (image.width() > *size || image.height() > *size))
    {
        image = image.resize(max_size, max_size, FilterType::Triangle);
    }
    let mut pixels = image.to_rgba8();
    apply_color_adjustments(&mut pixels, adjustments);
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
        let file = File::create(destination)?;
        let writer = BufWriter::new(file);
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

fn apply_color_adjustments(image: &mut RgbaImage, a: Adjustments) {
    if a.is_identity() {
        return;
    }
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
    let clarity = a.clarity / 100.0;
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
        let midtone_mask = 4.0 * luma * (1.0 - luma);
        for channel in &mut rgb {
            *channel = (*channel - 0.5) * contrast_factor + 0.5;
            *channel += (*channel - 0.5) * clarity * 0.22 * midtone_mask;
        }

        luma = luminance(rgb);
        let current_saturation = rgb.iter().copied().fold(f32::MIN, f32::max)
            - rgb.iter().copied().fold(f32::MAX, f32::min);
        let vibrance_factor = 1.0 + vibrance * (1.0 - current_saturation.clamp(0.0, 1.0));
        let saturation_factor = (1.0 + saturation) * vibrance_factor;
        for channel in &mut rgb {
            *channel = luma + (*channel - luma) * saturation_factor;
        }

        if vignette != 0.0 {
            let nx = (x as f32 + 0.5) / width * 2.0 - 1.0;
            let ny = (y as f32 + 0.5) / height * 2.0 - 1.0;
            let edge = ((nx * nx + ny * ny) / 2.0).clamp(0.0, 1.0).powf(1.4);
            let multiplier = (1.0 + vignette * edge * 0.72).max(0.0);
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

fn luminance(rgb: [f32; 3]) -> f32 {
    rgb[0] * 0.2126 + rgb[1] * 0.7152 + rgb[2] * 0.0722
}

#[cfg(test)]
mod tests {
    use image::{DynamicImage, Rgba, RgbaImage};

    use super::*;

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
    fn rotation_changes_dimensions() {
        let source = DynamicImage::new_rgba8(4, 2);
        let rendered = render_image(
            source,
            Adjustments {
                rotation: 90,
                ..Default::default()
            },
            RenderOptions::default(),
        );
        assert_eq!((rendered.width(), rendered.height()), (2, 4));
    }

    #[test]
    fn maximum_size_never_upscales() {
        let source = DynamicImage::new_rgba8(40, 20);
        let rendered = render_image(
            source,
            Adjustments::default(),
            RenderOptions {
                max_size: Some(200),
            },
        );
        assert_eq!((rendered.width(), rendered.height()), (40, 20));
    }
}
