use std::{
    error::Error,
    fmt,
    io::Cursor,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use image::{ImageFormat, RgbaImage};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use spectrum_imaging::{
    Adjustments, ExactRegionSource, PixelRegion, RegionRenderError,
    render_image_region_at_source_resolution_bounded,
};

use crate::{
    Layer, LayerKind, MAX_COLOR_SELECTION_PIXELS, MAX_PAINT_REGION_PIXELS, PixelMask,
    RasterSourceResolver, SequentialPngLimits, SequentialPngSource, Transform, VectorMask,
    validation::{validate_adjustments, validate_transform},
};

pub const SAMPLED_SOURCE_VERSION: u32 = 1;
pub const MAX_SAMPLED_SOURCE_NAME_CHARS: usize = 256;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SampledSourceSnapshot {
    pub version: u32,
    pub source_layer_id: u64,
    pub source_layer_name: String,
    pub path: PathBuf,
    pub content_hash: String,
    pub width: u32,
    pub height: u32,
    pub anchor_local: [f32; 2],
    pub adjustments: Adjustments,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pixel_mask: Option<PixelMask>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vector_mask: Option<VectorMask>,
}

impl SampledSourceSnapshot {
    pub(crate) fn capture(layer: &Layer, document_point: [f32; 2]) -> Result<Self> {
        let LayerKind::Raster { path, .. } = &layer.kind else {
            bail!("Clone Stamp source must be a Raster layer");
        };
        if !document_point[0].is_finite() || !document_point[1].is_finite() {
            bail!("Clone Stamp source point must be finite");
        }
        validate_transform(layer.transform)?;
        validate_non_geometric_adjustments(&layer.adjustments)?;
        let (canonical, bytes) = crate::font_source::read_secure_regular_file(
            path,
            crate::revisions::MAX_EMBEDDED_RASTER_BYTES,
            "Clone Stamp raster source",
        )
        .with_context(|| format!("could not capture Clone Stamp source {}", path.display()))?;
        let dimensions = image_dimensions_from_bytes(&bytes)?;
        let anchor_local = document_to_local(document_point, dimensions, layer.transform)
            .context("Clone Stamp source point cannot be mapped through the source transform")?;
        if anchor_local[0] < 0.0
            || anchor_local[1] < 0.0
            || anchor_local[0] >= dimensions.0 as f32
            || anchor_local[1] >= dimensions.1 as f32
        {
            bail!("Clone Stamp source point is outside the Raster source");
        }
        let snapshot = Self {
            version: SAMPLED_SOURCE_VERSION,
            source_layer_id: layer.id,
            source_layer_name: layer
                .name
                .chars()
                .take(MAX_SAMPLED_SOURCE_NAME_CHARS)
                .collect(),
            path: canonical,
            content_hash: hex_sha256(&bytes),
            width: dimensions.0,
            height: dimensions.1,
            anchor_local,
            adjustments: layer.adjustments.clone(),
            pixel_mask: layer.pixel_mask.clone(),
            vector_mask: layer.vector_mask.clone(),
        };
        snapshot.validate_metadata()?;
        Ok(snapshot)
    }

    pub(crate) fn validate_metadata(&self) -> Result<()> {
        if self.version != SAMPLED_SOURCE_VERSION {
            bail!(
                "unsupported sampled source version {} (expected {SAMPLED_SOURCE_VERSION})",
                self.version
            );
        }
        if self.source_layer_id == 0 {
            bail!("sampled source layer identity must be nonzero");
        }
        if self.source_layer_name.chars().count() > MAX_SAMPLED_SOURCE_NAME_CHARS {
            bail!("sampled source layer name exceeds its character limit");
        }
        if self.content_hash.len() != 64
            || !self
                .content_hash
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            bail!("sampled source content hash must be lowercase SHA-256");
        }
        if self.width == 0
            || self.height == 0
            || self.width > crate::MAX_CANVAS_DIMENSION
            || self.height > crate::MAX_CANVAS_DIMENSION
        {
            bail!("sampled source dimensions are outside Prism limits");
        }
        if !self.anchor_local[0].is_finite() || !self.anchor_local[1].is_finite() {
            bail!("sampled source anchor must be finite");
        }
        if self.anchor_local[0] < 0.0
            || self.anchor_local[1] < 0.0
            || self.anchor_local[0] >= self.width as f32
            || self.anchor_local[1] >= self.height as f32
        {
            bail!("sampled source anchor is outside its Raster source");
        }
        validate_non_geometric_adjustments(&self.adjustments)?;
        if let Some(mask) = &self.pixel_mask {
            let pixels = u64::from(mask.width) * u64::from(mask.height);
            if (mask.width, mask.height) != (self.width, self.height)
                || pixels > MAX_COLOR_SELECTION_PIXELS
                || usize::try_from(pixels).ok() != Some(mask.alpha.len())
                || !mask.has_valid_identity()
            {
                bail!("sampled source pixel mask is malformed or has mismatched dimensions");
            }
        }
        if let Some(mask) = &self.vector_mask {
            mask.validate()?;
        }
        Ok(())
    }

    pub(crate) fn validate_asset(&self) -> Result<()> {
        self.validate_metadata()?;
        let (_, bytes) = crate::font_source::read_secure_regular_file(
            &self.path,
            crate::revisions::MAX_EMBEDDED_RASTER_BYTES,
            "Clone Stamp raster source",
        )
        .with_context(|| {
            format!(
                "could not validate Clone Stamp source {}",
                self.path.display()
            )
        })?;
        self.validate_embedded_bytes(&bytes)
    }

    pub(crate) fn validate_embedded_bytes(&self, bytes: &[u8]) -> Result<()> {
        self.validate_metadata()?;
        if hex_sha256(bytes) != self.content_hash {
            bail!("Clone Stamp source bytes do not match their captured SHA-256");
        }
        if image_dimensions_from_bytes(bytes)? != (self.width, self.height) {
            bail!("Clone Stamp source dimensions changed after capture");
        }
        Ok(())
    }

    pub(crate) fn asset_path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn set_asset_path(&mut self, path: PathBuf) {
        self.path = path;
    }
}

pub(crate) fn sampled_source_region(
    source: &SampledSourceSnapshot,
    region: PixelRegion,
    raster_sources: Option<&dyn RasterSourceResolver>,
) -> Result<RgbaImage> {
    source.validate_metadata()?;
    if region.width == 0
        || region.height == 0
        || u64::from(region.width) * u64::from(region.height) > MAX_PAINT_REGION_PIXELS
        || region
            .x
            .checked_add(region.width)
            .is_none_or(|right| right > source.width)
        || region
            .y
            .checked_add(region.height)
            .is_none_or(|bottom| bottom > source.height)
    {
        bail!("sampled source region exceeds its bounded Raster source");
    }
    let result = render_image_region_at_source_resolution_bounded(
        source.width,
        source.height,
        source.adjustments.clone(),
        region,
        MAX_PAINT_REGION_PIXELS,
        |required| read_base_region(source, required, raster_sources),
    );
    let (mut image, _) = match result {
        Ok(rendered) => rendered,
        Err(RegionRenderError::ExceedsStagingPixelLimit) => {
            bail!("sampled source adjustments exceed the bounded staging limit")
        }
        Err(error) => return Err(anyhow::Error::new(error)),
    };
    crate::pixel_masks::apply_adjusted_pixel_mask_region(
        &mut image,
        source.pixel_mask.as_ref(),
        (source.width, source.height),
        &source.adjustments,
        (source.width, source.height),
        (region.x, region.y),
    )?;
    crate::paths::apply_vector_mask_to_image(
        &mut image,
        source.vector_mask.as_ref(),
        source.width,
        source.height,
        region.x,
        region.y,
    )?;
    Ok(image)
}

fn read_base_region(
    source: &SampledSourceSnapshot,
    region: PixelRegion,
    raster_sources: Option<&dyn RasterSourceResolver>,
) -> Result<RgbaImage, SourceReadError> {
    if let Some(provider) = raster_sources.and_then(|resolver| resolver.resolve(&source.path)) {
        let descriptor = &provider.source().info().descriptor;
        if (descriptor.width, descriptor.height) != (source.width, source.height) {
            return Err(SourceReadError::message(
                "resolved Clone Stamp provider dimensions do not match the captured source",
            ));
        }
        return provider
            .source()
            .read_exact_region(region)
            .map_err(SourceReadError::dynamic);
    }
    if raster_sources.is_some() {
        return Err(SourceReadError::message(
            "Clone Stamp source is not ready in the immutable raster resolver snapshot",
        ));
    }
    let inspection = crate::raster_region::inspect_raster_region_source(&source.path)
        .map_err(SourceReadError::anyhow)?;
    if (
        inspection.info.descriptor.width,
        inspection.info.descriptor.height,
    ) != (source.width, source.height)
    {
        return Err(SourceReadError::message(
            "Clone Stamp source dimensions changed after capture",
        ));
    }
    if inspection.format == ImageFormat::Png && inspection.info.supports_region_reads_now() {
        let sequential = SequentialPngSource::open(
            &source.path,
            SequentialPngLimits {
                max_encoded_source_bytes: crate::revisions::MAX_EMBEDDED_RASTER_BYTES as u64,
                max_region_pixels: MAX_PAINT_REGION_PIXELS,
            },
        )
        .map_err(SourceReadError::anyhow)?;
        if sequential.source_sha256() != source.content_hash {
            return Err(SourceReadError::message(
                "Clone Stamp source bytes do not match their captured SHA-256",
            ));
        }
        return sequential
            .read_exact_region(region)
            .map_err(SourceReadError::boxed);
    }

    let pixels = u64::from(source.width) * u64::from(source.height);
    if pixels > MAX_PAINT_REGION_PIXELS {
        return Err(SourceReadError::message(
            "Clone Stamp source requires a prepared bounded raster provider",
        ));
    }
    let (_, bytes) = crate::font_source::read_secure_regular_file(
        &source.path,
        crate::revisions::MAX_EMBEDDED_RASTER_BYTES,
        "Clone Stamp raster source",
    )
    .map_err(SourceReadError::anyhow)?;
    if hex_sha256(&bytes) != source.content_hash {
        return Err(SourceReadError::message(
            "Clone Stamp source bytes do not match their captured SHA-256",
        ));
    }
    let mut reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(SourceReadError::boxed)?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(crate::MAX_CANVAS_DIMENSION);
    limits.max_image_height = Some(crate::MAX_CANVAS_DIMENSION);
    limits.max_alloc = Some(MAX_PAINT_REGION_PIXELS * 8);
    reader.limits(limits);
    let decoded = reader.decode().map_err(SourceReadError::image)?.to_rgba8();
    Ok(
        image::imageops::crop_imm(&decoded, region.x, region.y, region.width, region.height)
            .to_image(),
    )
}

fn validate_non_geometric_adjustments(adjustments: &Adjustments) -> Result<()> {
    validate_adjustments(adjustments)?;
    if adjustments.rotation != 0
        || adjustments.flip_horizontal
        || adjustments.flip_vertical
        || adjustments.straighten.abs() > 0.01
        || adjustments.crop.is_some()
    {
        bail!(
            "Clone Stamp source cannot have crop, flip, rotation, or straighten adjustments; reset them before setting the source"
        );
    }
    Ok(())
}

fn document_to_local(
    point: [f32; 2],
    dimensions: (u32, u32),
    transform: Transform,
) -> Option<[f32; 2]> {
    if transform.scale_x <= 0.0 || transform.scale_y <= 0.0 {
        return None;
    }
    let center = [
        dimensions.0 as f32 * transform.scale_x * 0.5,
        dimensions.1 as f32 * transform.scale_y * 0.5,
    ];
    let dx = point[0] - transform.x - center[0];
    let dy = point[1] - transform.y - center[1];
    let radians = -transform.rotation.to_radians();
    let (sin, cos) = radians.sin_cos();
    Some([
        (dx * cos - dy * sin + center[0]) / transform.scale_x,
        (dx * sin + dy * cos + center[1]) / transform.scale_y,
    ])
}

fn image_dimensions_from_bytes(bytes: &[u8]) -> Result<(u32, u32)> {
    let reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .context("could not identify Clone Stamp raster source")?;
    let dimensions = reader
        .into_dimensions()
        .context("could not inspect Clone Stamp raster source")?;
    if dimensions.0 == 0
        || dimensions.1 == 0
        || dimensions.0 > crate::MAX_CANVAS_DIMENSION
        || dimensions.1 > crate::MAX_CANVAS_DIMENSION
    {
        bail!("Clone Stamp raster dimensions are outside Prism limits");
    }
    Ok(dimensions)
}

fn hex_sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut value = String::with_capacity(64);
    for byte in digest {
        use fmt::Write as _;
        write!(&mut value, "{byte:02x}").expect("writing into a String cannot fail");
    }
    value
}

#[derive(Debug)]
struct SourceReadError {
    message: String,
    source: Option<Box<dyn Error + Send + Sync>>,
}

impl SourceReadError {
    fn message(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            source: None,
        }
    }

    fn anyhow(error: anyhow::Error) -> Self {
        Self {
            message: error.to_string(),
            source: Some(error.into()),
        }
    }

    fn image(error: image::ImageError) -> Self {
        Self {
            message: error.to_string(),
            source: Some(Box::new(error)),
        }
    }

    fn boxed(error: impl Error + Send + Sync + 'static) -> Self {
        Self {
            message: error.to_string(),
            source: Some(Box::new(error)),
        }
    }

    fn dynamic(error: Box<dyn Error + Send + Sync>) -> Self {
        Self {
            message: error.to_string(),
            source: Some(error),
        }
    }
}

impl fmt::Display for SourceReadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for SourceReadError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.source.as_deref().map(|source| source as _)
    }
}
