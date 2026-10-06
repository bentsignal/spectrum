//! An image asset: one source photo and the edits made to it. The source
//! never changes; edits are settings applied whenever the image renders.
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use rawler::{RawLoader, decoders::RawDecodeParams, formats::tiff::Rational, rawsource::RawSource};
use serde::{Deserialize, Serialize};

use crate::Adjustments;

/// Camera details read when an image is imported.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ImageMetadata {
    pub camera_make: Option<String>,
    pub camera_model: Option<String>,
    pub lens: Option<String>,
    pub iso: Option<u32>,
    pub focal_length_mm: Option<f32>,
    pub aperture: Option<f32>,
    pub shutter_seconds: Option<f32>,
    pub captured_at: Option<String>,
}

/// An image: its source file and how it is edited.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Image {
    /// The source photo; inside a library, the image document's own copy.
    pub path: PathBuf,
    pub width: u32,
    pub height: u32,
    /// The source's file type, such as "jpg" or "arw".
    pub format: String,
    #[serde(default)]
    pub metadata: ImageMetadata,
    #[serde(default)]
    pub adjustments: Adjustments,
}

impl Image {
    /// An unedited image of a source whose size is known.
    pub fn new(path: PathBuf, width: u32, height: u32) -> Self {
        Self {
            format: extension(&path),
            path,
            width,
            height,
            metadata: ImageMetadata::default(),
            adjustments: Adjustments::default(),
        }
    }

    /// Reads a photo's size and camera details, unedited.
    pub fn import(path: &Path) -> Result<Self> {
        if !is_supported_image(path) {
            bail!("unsupported image type: {}", path.display());
        }
        let path = std::fs::canonicalize(path)
            .with_context(|| format!("could not find image {}", path.display()))?;
        let (width, height, metadata) = source_info(&path)?;
        Ok(Self {
            format: extension(&path),
            path,
            width,
            height,
            metadata,
            adjustments: Adjustments::default(),
        })
    }

    pub fn is_raw(&self) -> bool {
        is_raw_image(&self.path)
    }
}

fn source_info(path: &Path) -> Result<(u32, u32, ImageMetadata)> {
    if !is_raw_image(path) {
        let (width, height) = image::ImageReader::open(path)
            .with_context(|| format!("could not open {}", path.display()))?
            .with_guessed_format()?
            .into_dimensions()
            .with_context(|| format!("could not read {}", path.display()))?;
        return Ok((width, height, ImageMetadata::default()));
    }
    let source = RawSource::new(path)
        .with_context(|| format!("could not open Sony RAW {}", path.display()))?;
    let loader = RawLoader::new();
    let decoder = loader
        .get_decoder(&source)
        .with_context(|| format!("could not inspect Sony RAW {}", path.display()))?;
    let params = RawDecodeParams::default();
    // A dummy decode reads the dimensions and checks the container without
    // allocating or demosaicing the pixels.
    let raw = decoder
        .raw_image(&source, &params, true)
        .with_context(|| format!("could not inspect Sony RAW {}", path.display()))?;
    let raw_metadata = decoder
        .raw_metadata(&source, &params)
        .with_context(|| format!("could not read Sony RAW metadata {}", path.display()))?;
    let transpose = raw.orientation.to_flips().0;
    let (width, height) = raw.crop_area.or(raw.active_area).map_or_else(
        || (raw.width as u32, raw.height as u32),
        |area| (area.d.w as u32, area.d.h as u32),
    );
    let exif = raw_metadata.exif;
    let lens = exif.lens_model.clone().or_else(|| {
        raw_metadata
            .lens
            .as_ref()
            .map(|lens| lens.lens_name.clone())
    });
    let metadata = ImageMetadata {
        camera_make: Some(raw_metadata.make),
        camera_model: Some(raw_metadata.model),
        lens,
        iso: exif
            .iso_speed
            .or(exif.recommended_exposure_index)
            .or(exif.iso_speed_ratings.map(u32::from)),
        focal_length_mm: rational(exif.focal_length),
        aperture: rational(exif.fnumber.or(exif.aperture_value)),
        shutter_seconds: rational(exif.exposure_time),
        captured_at: exif.date_time_original.or(exif.create_date),
    };
    Ok(if transpose {
        (height, width, metadata)
    } else {
        (width, height, metadata)
    })
}

fn rational(value: Option<Rational>) -> Option<f32> {
    value.and_then(|value| (value.d != 0).then_some(value.n as f32 / value.d as f32))
}

pub fn is_raw_image(path: &Path) -> bool {
    extension(path) == "arw"
}

pub fn is_supported_image(path: &Path) -> bool {
    matches!(
        extension(path).as_str(),
        "jpg" | "jpeg" | "png" | "tif" | "tiff" | "webp" | "arw"
    )
}

fn extension(path: &Path) -> String {
    path.extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
}
