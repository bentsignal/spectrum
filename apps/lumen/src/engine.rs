use std::{
    fs::File,
    io::BufWriter,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use image::{DynamicImage, GrayImage, ImageEncoder, RgbImage, RgbaImage, imageops::FilterType};
use rawler::{
    Orientation,
    decoders::RawDecodeParams,
    imgop::develop::{Intermediate, RawDevelop},
};

use crate::{Adjustments, Photo, project::is_raw_image};
pub use spectrum_imaging::{ExportFormat, RenderOptions, render_image};

const MAX_AUTHORITATIVE_RAW_PIXELS: u64 = 25_000_000;
const RAW_DEVELOP_BUDGET_BYTES_PER_PIXEL: u64 = 48;
const MAX_AUTHORITATIVE_RAW_WORKING_BYTES: u64 =
    MAX_AUTHORITATIVE_RAW_PIXELS * RAW_DEVELOP_BUDGET_BYTES_PER_PIXEL;

trait PhotoDecoder {
    fn authoritative(
        &self,
        photo: &Photo,
        purpose: AuthoritativeDecodePurpose,
    ) -> Result<DynamicImage>;
    fn proxy(&self, photo: &Photo, max_size: u32) -> Result<DynamicImage>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AuthoritativeDecodePurpose {
    SettledPreview,
    Export,
}

struct FilePhotoDecoder;

impl PhotoDecoder for FilePhotoDecoder {
    fn authoritative(
        &self,
        photo: &Photo,
        purpose: AuthoritativeDecodePurpose,
    ) -> Result<DynamicImage> {
        if is_raw_image(&photo.path) {
            develop_raw(photo, purpose)
        } else {
            decode_raster(photo)
        }
    }

    fn proxy(&self, photo: &Photo, max_size: u32) -> Result<DynamicImage> {
        if !is_raw_image(&photo.path) {
            return decode_raster(photo);
        }
        let params = RawDecodeParams::default();
        let embedded = if max_size <= 320 {
            rawler::analyze::extract_thumbnail_pixels(&photo.path, &params)
        } else {
            rawler::analyze::extract_preview_pixels(&photo.path, &params)
        };
        match embedded {
            Ok(preview) => Ok(orient_embedded_preview(preview, photo)),
            Err(_) => self.authoritative(photo, AuthoritativeDecodePurpose::SettledPreview),
        }
    }
}

pub fn render_photo(photo: &Photo, options: RenderOptions) -> Result<DynamicImage> {
    render_photo_with_adjustments(photo, photo.adjustments.clone(), options)
}

pub fn render_photo_with_adjustments(
    photo: &Photo,
    adjustments: Adjustments,
    options: RenderOptions,
) -> Result<DynamicImage> {
    render_photo_with_decoder(photo, adjustments, options, &FilePhotoDecoder)
}

/// Produce the settled, export-authoritative preview raster.
///
/// Geometry is applied at source resolution before the long-edge limit, exactly
/// as it is for a size-limited export.
pub fn render_settled_preview(
    photo: &Photo,
    adjustments: Adjustments,
    max_size: u32,
) -> Result<DynamicImage> {
    render_settled_preview_with_decoder(photo, adjustments, max_size, &FilePhotoDecoder)
}

fn render_settled_preview_with_decoder(
    photo: &Photo,
    adjustments: Adjustments,
    max_size: u32,
    decoder: &impl PhotoDecoder,
) -> Result<DynamicImage> {
    render_with_decoder(
        photo,
        adjustments,
        RenderOptions {
            max_size: Some(max_size),
        },
        decoder,
        AuthoritativeDecodePurpose::SettledPreview,
    )
}

fn render_photo_with_decoder(
    photo: &Photo,
    adjustments: Adjustments,
    options: RenderOptions,
    decoder: &impl PhotoDecoder,
) -> Result<DynamicImage> {
    render_with_decoder(
        photo,
        adjustments,
        options,
        decoder,
        AuthoritativeDecodePurpose::Export,
    )
}

fn render_with_decoder(
    photo: &Photo,
    adjustments: Adjustments,
    options: RenderOptions,
    decoder: &impl PhotoDecoder,
    purpose: AuthoritativeDecodePurpose,
) -> Result<DynamicImage> {
    let decoded = decoder.authoritative(photo, purpose)?;
    Ok(render_image(decoded, adjustments, options))
}

/// Decode immutable original pixels and optionally downsample the decoded source.
///
/// RAW inputs are always developed through the same path used by export. Call
/// [`render_settled_preview`] when `max_size` must be applied after geometry.
pub fn decode_photo(photo: &Photo, max_size: Option<u32>) -> Result<DynamicImage> {
    decode_photo_with_decoder(photo, max_size, &FilePhotoDecoder)
}

fn decode_photo_with_decoder(
    photo: &Photo,
    max_size: Option<u32>,
    decoder: &impl PhotoDecoder,
) -> Result<DynamicImage> {
    let purpose = if max_size.is_some() {
        AuthoritativeDecodePurpose::SettledPreview
    } else {
        AuthoritativeDecodePurpose::Export
    };
    Ok(resize_to_limit(
        decoder.authoritative(photo, purpose)?,
        max_size,
    ))
}

/// Decode a display-only proxy for places where responsiveness matters more than
/// export fidelity, such as the filmstrip. RAW proxies may use the camera's
/// embedded rendering and must never be used as a settled develop preview or an
/// export source.
pub fn decode_photo_proxy(photo: &Photo, max_size: u32) -> Result<DynamicImage> {
    decode_photo_proxy_with_decoder(photo, max_size, &FilePhotoDecoder)
}

fn decode_photo_proxy_with_decoder(
    photo: &Photo,
    max_size: u32,
    decoder: &impl PhotoDecoder,
) -> Result<DynamicImage> {
    Ok(resize_to_limit(
        decoder.proxy(photo, max_size)?,
        Some(max_size),
    ))
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

/// Render already-decoded working pixels through the shared adjustment path.
///
/// This helper does not apply a post-geometry size limit. It is suitable for a
/// transient interaction source, not the settled export oracle.
pub fn render_preview_source(source: DynamicImage, adjustments: Adjustments) -> DynamicImage {
    render_image(source, adjustments, RenderOptions::default())
}

fn develop_raw(photo: &Photo, purpose: AuthoritativeDecodePurpose) -> Result<DynamicImage> {
    if purpose == AuthoritativeDecodePurpose::SettledPreview {
        validate_raw_develop_budget(photo.width as u64, photo.height as u64, &photo.path)?;
    }
    let raw = rawler::decode_file(&photo.path)
        .with_context(|| format!("could not decode Sony RAW {}", photo.path.display()))?;
    if purpose == AuthoritativeDecodePurpose::SettledPreview {
        validate_raw_develop_budget(raw.width as u64, raw.height as u64, &photo.path)?;
    }
    let orientation = raw.orientation;
    let intermediate = RawDevelop::default()
        .develop_intermediate(&raw)
        .with_context(|| format!("could not develop Sony RAW {}", photo.path.display()))?;
    drop(raw);
    let developed = intermediate_to_dynamic_u8(intermediate)
        .with_context(|| format!("Sony RAW produced no image: {}", photo.path.display()))?;
    Ok(apply_orientation(developed, orientation))
}

fn validate_raw_develop_budget(width: u64, height: u64, path: &Path) -> Result<()> {
    if width == 0 || height == 0 {
        return Ok(());
    }
    let pixels = width
        .checked_mul(height)
        .context("RAW dimensions exceed the supported range")?;
    if pixels > MAX_AUTHORITATIVE_RAW_PIXELS {
        bail!(
            "RAW {} is {width}x{height} ({pixels} pixels), above Lumen's {}-pixel authoritative development limit ({} MiB working budget)",
            path.display(),
            MAX_AUTHORITATIVE_RAW_PIXELS,
            MAX_AUTHORITATIVE_RAW_WORKING_BYTES / (1024 * 1024),
        );
    }
    Ok(())
}

fn intermediate_to_dynamic_u8(intermediate: Intermediate) -> Option<DynamicImage> {
    match intermediate {
        Intermediate::Monochrome(pixels) => {
            let data = pixels
                .data
                .into_iter()
                .map(raw_float_to_u8)
                .collect::<Vec<_>>();
            GrayImage::from_raw(pixels.width as u32, pixels.height as u32, data)
                .map(DynamicImage::ImageLuma8)
        }
        Intermediate::ThreeColor(pixels) => {
            let mut data = Vec::with_capacity(pixels.data.len() * 3);
            for pixel in pixels.data {
                data.extend(pixel.map(raw_float_to_u8));
            }
            RgbImage::from_raw(pixels.width as u32, pixels.height as u32, data)
                .map(DynamicImage::ImageRgb8)
        }
        Intermediate::FourColor(pixels) => {
            let mut data = Vec::with_capacity(pixels.data.len() * 4);
            for pixel in pixels.data {
                data.extend(pixel.map(raw_float_to_u8));
            }
            RgbaImage::from_raw(pixels.width as u32, pixels.height as u32, data)
                .map(DynamicImage::ImageRgba8)
        }
    }
}

fn raw_float_to_u8(value: f32) -> u8 {
    ((value.clamp(0.0, 1.0) * u16::MAX as f32) as u16 / 257) as u8
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
    if export_destination_aliases_source(&photo.path, destination) {
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

fn export_destination_aliases_source(source: &Path, destination: &Path) -> bool {
    source == destination || same_file::is_same_file(source, destination).unwrap_or(false)
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
        cell::{Cell, RefCell},
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

    struct ObservableRawDecoder {
        authoritative_calls: Cell<usize>,
        proxy_calls: Cell<usize>,
        purposes: RefCell<Vec<AuthoritativeDecodePurpose>>,
    }

    impl ObservableRawDecoder {
        fn new() -> Self {
            Self {
                authoritative_calls: Cell::new(0),
                proxy_calls: Cell::new(0),
                purposes: RefCell::new(Vec::new()),
            }
        }
    }

    impl PhotoDecoder for ObservableRawDecoder {
        fn authoritative(
            &self,
            _photo: &Photo,
            purpose: AuthoritativeDecodePurpose,
        ) -> Result<DynamicImage> {
            self.authoritative_calls
                .set(self.authoritative_calls.get() + 1);
            self.purposes.borrow_mut().push(purpose);
            Ok(DynamicImage::ImageRgba8(RgbaImage::from_pixel(
                8,
                6,
                Rgba([214, 42, 17, 255]),
            )))
        }

        fn proxy(&self, _photo: &Photo, _max_size: u32) -> Result<DynamicImage> {
            self.proxy_calls.set(self.proxy_calls.get() + 1);
            Ok(DynamicImage::ImageRgba8(RgbaImage::from_pixel(
                8,
                6,
                Rgba([13, 71, 229, 255]),
            )))
        }
    }

    struct SixThousandByFourThousandDecoder;

    impl PhotoDecoder for SixThousandByFourThousandDecoder {
        fn authoritative(
            &self,
            _photo: &Photo,
            _purpose: AuthoritativeDecodePurpose,
        ) -> Result<DynamicImage> {
            Ok(DynamicImage::new_rgb8(6_000, 4_000))
        }

        fn proxy(&self, _photo: &Photo, _max_size: u32) -> Result<DynamicImage> {
            unreachable!("dimension test must use authoritative pixels")
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
    fn raw_settled_preview_and_export_use_authoritative_pixels_while_thumbnail_uses_proxy() {
        let photo = Photo::new(1, "observable.arw".into(), "observable.arw".into(), 8, 6);
        let decoder = ObservableRawDecoder::new();
        let adjustments = Adjustments {
            exposure: 0.25,
            ..Default::default()
        };

        let settled =
            render_settled_preview_with_decoder(&photo, adjustments.clone(), 8, &decoder).unwrap();
        let exported = render_photo_with_decoder(
            &photo,
            adjustments,
            RenderOptions { max_size: Some(8) },
            &decoder,
        )
        .unwrap();
        let thumbnail = decode_photo_proxy_with_decoder(&photo, 8, &decoder).unwrap();

        assert_eq!(settled.to_rgba8(), exported.to_rgba8());
        assert_ne!(settled.to_rgba8(), thumbnail.to_rgba8());
        assert_eq!(decoder.authoritative_calls.get(), 2);
        assert_eq!(decoder.proxy_calls.get(), 1);
        assert_eq!(
            *decoder.purposes.borrow(),
            [
                AuthoritativeDecodePurpose::SettledPreview,
                AuthoritativeDecodePurpose::Export,
            ]
        );
        assert!(settled.to_rgba8().get_pixel(0, 0)[0] > 214);
        assert_eq!(
            thumbnail.to_rgba8().get_pixel(0, 0),
            &Rgba([13, 71, 229, 255])
        );
    }

    #[test]
    fn max_size_is_applied_after_crop_geometry() {
        let photo = Photo::new(
            1,
            "dimensions.png".into(),
            "dimensions.png".into(),
            6_000,
            4_000,
        );
        let uncropped = render_photo_with_decoder(
            &photo,
            Adjustments::default(),
            RenderOptions {
                max_size: Some(1_800),
            },
            &SixThousandByFourThousandDecoder,
        )
        .unwrap();
        assert_eq!(uncropped.dimensions(), (1_800, 1_200));
        drop(uncropped);

        let cropped = render_photo_with_decoder(
            &photo,
            Adjustments {
                crop: Some(CropRect {
                    x: 0.4,
                    y: 0.0,
                    width: 0.2,
                    height: 1.0,
                }),
                ..Default::default()
            },
            RenderOptions {
                max_size: Some(1_800),
            },
            &SixThousandByFourThousandDecoder,
        )
        .unwrap();
        assert_eq!(cropped.dimensions(), (540, 1_800));
    }

    #[test]
    fn authoritative_raw_development_has_a_strict_working_set_plan() {
        validate_raw_develop_budget(6_048, 4_024, Path::new("a6400.arw")).unwrap();
        let error =
            validate_raw_develop_budget(8_000, 5_333, Path::new("oversize.arw")).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("25000000-pixel authoritative development limit")
        );
        assert_eq!(MAX_AUTHORITATIVE_RAW_WORKING_BYTES, 1_200_000_000);
    }

    #[test]
    #[ignore = "manual release-mode probe requiring LUMEN_RAW_PREVIEW_SAMPLE"]
    fn authoritative_raw_preview_memory_probe() {
        let path = std::env::var_os("LUMEN_RAW_PREVIEW_SAMPLE")
            .map(PathBuf::from)
            .expect("set LUMEN_RAW_PREVIEW_SAMPLE to an immutable 6000x4000 Sony ARW");
        let original_metadata = fs::metadata(&path).unwrap();
        let photo = Photo::new(
            1,
            path.clone(),
            path.file_name().unwrap().to_string_lossy().into_owned(),
            6_000,
            4_000,
        );

        let preview = render_settled_preview(&photo, Adjustments::default(), 1_800).unwrap();

        assert_eq!(preview.dimensions(), (1_800, 1_200));
        let final_metadata = fs::metadata(path).unwrap();
        assert_eq!(final_metadata.len(), original_metadata.len());
        assert_eq!(
            final_metadata.modified().unwrap(),
            original_metadata.modified().unwrap()
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
            let preview = render_settled_preview(&photo, adjustments, 29)
                .unwrap()
                .to_rgba8();
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

    #[test]
    fn export_rejects_a_hard_link_alias_without_changing_original_bytes() {
        let directory = TestDirectory::new("hard-link-export");
        let original_path = directory.0.join("original.png");
        let hard_link_path = directory.0.join("export.png");
        RgbaImage::from_pixel(4, 3, Rgba([41, 89, 137, 255]))
            .save(&original_path)
            .unwrap();
        fs::hard_link(&original_path, &hard_link_path).unwrap();
        let original_bytes = fs::read(&original_path).unwrap();
        let photo = Photo::new(1, original_path.clone(), "original.png".into(), 4, 3);

        let error =
            export_photo(&photo, &hard_link_path, RenderOptions::default(), 100).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("export destination cannot overwrite the original photo")
        );
        assert_eq!(fs::read(&original_path).unwrap(), original_bytes);
        assert_eq!(fs::read(&hard_link_path).unwrap(), original_bytes);
    }
}
