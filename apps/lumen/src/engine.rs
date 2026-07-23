use std::{
    fs::File,
    io::BufWriter,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use image::{DynamicImage, ImageEncoder, RgbaImage, imageops::FilterType};
use rawler::{Orientation, decoders::RawDecodeParams, imgop::develop::RawDevelop};

use crate::{Adjustments, Photo, project::is_raw_image};
pub use spectrum_imaging::{ExportFormat, RenderOptions, render_image};

pub fn render_photo(photo: &Photo, options: RenderOptions) -> Result<DynamicImage> {
    let decoded = decode_photo(photo, options.max_size)?;
    Ok(render_preview_source(decoded, photo.adjustments.clone()))
}

/// Decode immutable original pixels and optionally downsample them.
///
/// RAW inputs are always developed through the same path used by export. This is
/// the source contract for settled previews: rendering this raster with the
/// photo's adjustments is pixel-identical to a lossless export with the same
/// `max_size`, before texture upload or file encoding.
pub fn decode_photo(photo: &Photo, max_size: Option<u32>) -> Result<DynamicImage> {
    let decoded = if is_raw_image(&photo.path) {
        develop_raw(photo)?
    } else {
        decode_raster(photo)?
    };
    Ok(resize_to_limit(decoded, max_size))
}

/// Decode a display-only proxy for places where responsiveness matters more than
/// export fidelity, such as the filmstrip. RAW proxies may use the camera's
/// embedded rendering and must never be used as a settled develop preview or an
/// export source.
pub fn decode_photo_proxy(photo: &Photo, max_size: u32) -> Result<DynamicImage> {
    let decoded = if is_raw_image(&photo.path) {
        let params = RawDecodeParams::default();
        let embedded = if max_size <= 320 {
            rawler::analyze::extract_thumbnail_pixels(&photo.path, &params)
        } else {
            rawler::analyze::extract_preview_pixels(&photo.path, &params)
        };
        match embedded {
            Ok(preview) => orient_embedded_preview(preview, photo),
            Err(_) => develop_raw(photo)?,
        }
    } else {
        decode_raster(photo)?
    };
    Ok(resize_to_limit(decoded, Some(max_size)))
}

fn decode_raster(photo: &Photo) -> Result<DynamicImage> {
    image::ImageReader::open(&photo.path)
        .with_context(|| format!("could not open {}", photo.path.display()))?
        .with_guessed_format()?
        .decode()
        .with_context(|| format!("could not decode {}", photo.path.display()))
}

fn resize_to_limit(mut decoded: DynamicImage, max_size: Option<u32>) -> DynamicImage {
    if let Some(max_size) =
        max_size.filter(|size| *size > 0 && (decoded.width() > *size || decoded.height() > *size))
    {
        decoded = decoded.resize(max_size, max_size, FilterType::Triangle);
    }
    decoded
}

/// Render already-decoded preview pixels with the same adjustment path as export.
///
/// Settled previews and size-limited exports deliberately resize immutable
/// source pixels before applying adjustments. A full-resolution export
/// subsequently resampled by another viewer can differ slightly because
/// interpolation, sharpening, noise reduction, and spot radii operate in
/// different pixel spaces. JPEG loss and OS/GPU color management are also
/// outside this raster oracle.
pub fn render_preview_source(source: DynamicImage, adjustments: Adjustments) -> DynamicImage {
    render_image(source, adjustments, RenderOptions::default())
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
    use crate::{ColorGrade, CropRect, CurvePoint, SpotRemoval, ToneCurve, ToneCurves};
    use image::{GenericImageView, Rgba};
    use std::{
        fs,
        sync::atomic::{AtomicU64, Ordering},
    };

    static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(1);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new(label: &str) -> Self {
            let unique = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "lumen-engine-{label}-{}-{unique}",
                std::process::id()
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn parity_adjustments() -> Vec<(&'static str, Adjustments)> {
        let light = Adjustments {
            exposure: 0.65,
            contrast: 24.0,
            highlights: -31.0,
            shadows: 37.0,
            whites: 12.0,
            blacks: -9.0,
            ..Default::default()
        };

        let mut color = Adjustments {
            temperature: 18.0,
            tint: -11.0,
            vibrance: 27.0,
            saturation: -8.0,
            color_grading: crate::ColorGrading {
                shadows: ColorGrade {
                    hue: 218.0,
                    saturation: 22.0,
                    luminance: -4.0,
                },
                midtones: ColorGrade {
                    hue: 34.0,
                    saturation: 17.0,
                    luminance: 3.0,
                },
                balance: -14.0,
                ..Default::default()
            },
            curves: ToneCurves {
                master: ToneCurve {
                    points: vec![
                        CurvePoint { x: 0.0, y: 0.02 },
                        CurvePoint { x: 0.42, y: 0.36 },
                        CurvePoint { x: 1.0, y: 0.97 },
                    ],
                },
                blue: ToneCurve {
                    points: vec![
                        CurvePoint { x: 0.0, y: 0.05 },
                        CurvePoint { x: 1.0, y: 0.94 },
                    ],
                },
                ..Default::default()
            },
            ..Default::default()
        };
        color.hsl.blue.hue = 19.0;
        color.hsl.blue.saturation = -23.0;
        color.hsl.orange.luminance = 14.0;

        let geometry_and_detail = Adjustments {
            crop: Some(CropRect {
                x: 0.08,
                y: 0.11,
                width: 0.81,
                height: 0.76,
            }),
            rotation: 90,
            flip_horizontal: true,
            straighten: 3.25,
            texture: 16.0,
            clarity: -12.0,
            dehaze: 9.0,
            sharpening: 34.0,
            noise_reduction: 21.0,
            vignette: -18.0,
            spots: vec![SpotRemoval {
                x: 0.48,
                y: 0.52,
                radius: 0.07,
                opacity: 0.82,
            }],
            ..Default::default()
        };

        vec![
            ("light", light),
            ("color-hsl-curves-grading", color),
            ("geometry-detail", geometry_and_detail),
        ]
    }

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

    #[test]
    fn settled_preview_is_exact_lossless_export_oracle_and_preserves_original() {
        let directory = TestDirectory::new("preview-export-parity");
        let original_path = directory.0.join("original.png");
        let source = RgbaImage::from_fn(47, 31, |x, y| {
            Rgba([
                ((x * 5 + y * 3) % 256) as u8,
                ((x * 2 + y * 7) % 256) as u8,
                ((x * 11 + y * 13) % 256) as u8,
                255,
            ])
        });
        source.save(&original_path).unwrap();
        let original_bytes = fs::read(&original_path).unwrap();
        let mut photo = Photo::new(1, original_path.clone(), "original.png".into(), 47, 31);

        for (name, adjustments) in parity_adjustments() {
            photo.adjustments = adjustments.clone();
            let preview_source = decode_photo(&photo, Some(29)).unwrap();
            let preview = render_preview_source(preview_source, adjustments).to_rgba8();
            let export_path = directory.0.join(format!("{name}.png"));
            export_photo(
                &photo,
                &export_path,
                RenderOptions { max_size: Some(29) },
                100,
            )
            .unwrap();
            let exported = image::open(export_path).unwrap().to_rgba8();

            assert_eq!(
                preview, exported,
                "{name} settled preview diverged from its lossless export"
            );
        }

        assert_eq!(
            fs::read(original_path).unwrap(),
            original_bytes,
            "rendering and export must leave immutable originals untouched"
        );
    }
}
