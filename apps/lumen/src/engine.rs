use std::{
    fs::File,
    io::BufWriter,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use image::{DynamicImage, ImageEncoder, RgbaImage, imageops::FilterType};
use rawler::{Orientation, decoders::RawDecodeParams, imgop::develop::RawDevelop};

use crate::{Photo, project::is_raw_image};
pub use spectrum_imaging::{ExportFormat, RenderOptions, render_image};

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
        if let Some(limit) = max_size {
            let params = RawDecodeParams::default();
            let embedded = if limit <= 320 {
                rawler::analyze::extract_thumbnail_pixels(&photo.path, &params)
            } else {
                rawler::analyze::extract_preview_pixels(&photo.path, &params)
            };
            match embedded {
                Ok(preview) => orient_embedded_preview(preview, photo),
                Err(_) => develop_raw(photo)?,
            }
        } else {
            develop_raw(photo)?
        }
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

fn develop_raw(photo: &Photo) -> Result<DynamicImage> {
    let raw = rawler::decode_file(&photo.path)
        .with_context(|| format!("could not decode Sony RAW {}", photo.path.display()))?;
    let orientation = raw.orientation;
    let developed = RawDevelop::default()
        .develop_intermediate(&raw)
        .with_context(|| format!("could not develop Sony RAW {}", photo.path.display()))?
        .to_dynamic_image()
        .with_context(|| format!("Sony RAW produced no image: {}", photo.path.display()))?;
    Ok(apply_orientation(developed, orientation))
}

fn orient_embedded_preview(image: DynamicImage, photo: &Photo) -> DynamicImage {
    let expected_landscape = photo.width >= photo.height;
    let actual_landscape = image.width() >= image.height();
    if expected_landscape != actual_landscape {
        image.rotate90()
    } else {
        image
    }
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

pub fn export_photo(
    photo: &Photo,
    destination: &Path,
    options: RenderOptions,
    jpeg_quality: u8,
) -> Result<()> {
    if destination == photo.path
        || (destination.exists()
            && std::fs::canonicalize(destination).is_ok_and(|path| path == photo.path))
    {
        bail!("export destination cannot overwrite the original photo");
    }
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

pub fn batch_destination(photo: &Photo, directory: &Path, format: ExportFormat) -> PathBuf {
    let stem = photo.path.file_stem().unwrap_or_default().to_string_lossy();
    directory.join(format!("{stem}-lumen-{}.{}", photo.id, format.extension()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::GenericImageView;

    #[test]
    fn batch_names_are_unique_for_same_named_sources() {
        let first = Photo::new(4, "a/frame.arw".into(), "frame.arw".into(), 1, 1);
        let second = Photo::new(9, "b/frame.arw".into(), "frame.arw".into(), 1, 1);
        let directory = Path::new("exports");
        assert_ne!(
            batch_destination(&first, directory, ExportFormat::Jpeg),
            batch_destination(&second, directory, ExportFormat::Jpeg)
        );
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
