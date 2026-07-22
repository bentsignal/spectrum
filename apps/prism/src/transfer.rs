use std::{fs, path::PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::{
    Document, FontAsset, FontSlant, Layer, LayerKind, MAX_CANVAS_DIMENSION,
    effects::{validate_layer_style, validate_shape_fill},
    validation::{
        require_finite, validate_adjustments, validate_mask, validate_shape_stroke,
        validate_transform,
    },
};

pub const LAYER_TRANSFER_FORMAT: &str = "spectrum.prism.layer";
pub const LAYER_TRANSFER_VERSION: u32 = 4;
const MAX_LAYER_TRANSFER_JSON_BYTES: usize = 24 * 1024 * 1024;

/// A portable, single-layer payload for clipboard and cross-document transfer.
///
/// Document-local layer and font IDs and source-provenance paths are deliberately
/// removed. Only active raster and font paths remain so durable Prism revisions
/// can embed their bytes before recording `Command::InsertLayer`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LayerTransfer {
    pub format: String,
    pub version: u32,
    pub layer: Layer,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_asset: Option<LayerTransferFont>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LayerTransferFont {
    pub family: String,
    pub style: String,
    pub weight: u16,
    pub slant: FontSlant,
    pub source_name: String,
    pub subset_allowed: bool,
    pub content_hash: String,
    pub path: PathBuf,
}

impl LayerTransfer {
    pub fn from_selected(document: &Document) -> Result<Self> {
        let id = document.selected.context("no layer is selected")?;
        Self::from_document(document, id)
    }

    pub fn from_document(document: &Document, id: u64) -> Result<Self> {
        let mut layer = document.layer(id)?.clone();
        layer.id = 0;
        let font_asset = match &mut layer.kind {
            LayerKind::Raster { original_path, .. } => {
                *original_path = None;
                None
            }
            LayerKind::Text { typography, .. } => match typography.font_id.take() {
                Some(font_id) => Some(LayerTransferFont::from(document.font_asset(font_id)?)),
                None => None,
            },
            _ => None,
        };
        let version = if layer.vector_mask.is_some() || matches!(layer.kind, LayerKind::Path { .. })
        {
            LAYER_TRANSFER_VERSION
        } else if layer.pixel_mask.is_some() {
            3
        } else if !layer.style.is_empty() || layer.shape_fill.is_some() {
            2
        } else {
            1
        };
        Ok(Self {
            format: LAYER_TRANSFER_FORMAT.into(),
            version,
            layer,
            font_asset,
        })
    }

    pub fn to_json(&self) -> Result<String> {
        self.validate_envelope()?;
        serde_json::to_string(self).context("could not encode Prism layer transfer")
    }

    pub fn to_json_pretty(&self) -> Result<String> {
        self.validate_envelope()?;
        serde_json::to_string_pretty(self).context("could not encode Prism layer transfer")
    }

    pub fn from_json(value: &str) -> Result<Self> {
        if value.len() > MAX_LAYER_TRANSFER_JSON_BYTES {
            bail!("Prism layer transfer exceeds the 24 MiB metadata limit");
        }
        let transfer: Self =
            serde_json::from_str(value).context("invalid Prism layer transfer JSON")?;
        transfer.validate_envelope()?;
        Ok(transfer)
    }

    pub(crate) fn insert_into(self, document: &mut Document, index: Option<usize>) -> Result<u64> {
        self.validate_envelope()?;
        let Self {
            mut layer,
            font_asset,
            ..
        } = self;
        sanitize_layer(&mut layer)?;

        let transferred_font_id = font_asset
            .map(|font| import_transferred_font(document, font))
            .transpose()?;
        match &mut layer.kind {
            LayerKind::Text { typography, .. } => typography.font_id = transferred_font_id,
            _ if transferred_font_id.is_some() => {
                unreachable!("the transfer envelope rejects fonts on non-text layers")
            }
            _ => {}
        }

        let id = document.allocate_id();
        layer.id = id;
        let default_index = document
            .selected
            .and_then(|selected| {
                document
                    .layers
                    .iter()
                    .position(|candidate| candidate.id == selected)
                    .map(|position| position + 1)
            })
            .unwrap_or(document.layers.len());
        document.layers.insert(
            index.unwrap_or(default_index).min(document.layers.len()),
            layer,
        );
        document.selected = Some(id);
        Ok(id)
    }

    pub(crate) fn validate_envelope(&self) -> Result<()> {
        if self.format != LAYER_TRANSFER_FORMAT {
            bail!("unsupported Prism layer transfer format {}", self.format);
        }
        if !(1..=LAYER_TRANSFER_VERSION).contains(&self.version) {
            bail!(
                "unsupported Prism layer transfer version {} (supports 1 through {LAYER_TRANSFER_VERSION})",
                self.version
            );
        }
        if self.version == 1 && (!self.layer.style.is_empty() || self.layer.shape_fill.is_some()) {
            bail!("Prism layer transfer version 1 cannot contain layer styles or shape fills");
        }
        if self.version < 3 && self.layer.pixel_mask.is_some() {
            bail!("Prism layer transfer versions before 3 cannot contain pixel masks");
        }
        if self.version < 4
            && (self.layer.vector_mask.is_some()
                || matches!(self.layer.kind, LayerKind::Path { .. }))
        {
            bail!("Prism layer transfer versions before 4 cannot contain paths or vector masks");
        }
        if self.layer.id != 0 {
            bail!("Prism layer transfers cannot contain a document-local layer ID");
        }
        validate_pixel_mask(&self.layer)?;
        match &self.layer.kind {
            LayerKind::Text { typography, .. } => {
                if typography.font_id.is_some() {
                    bail!("Prism layer transfers cannot contain a document-local font ID");
                }
            }
            _ if self.font_asset.is_some() => {
                bail!("only a text layer transfer can include a font asset");
            }
            _ => {}
        }
        Ok(())
    }
}

impl From<&FontAsset> for LayerTransferFont {
    fn from(font: &FontAsset) -> Self {
        Self {
            family: font.family.clone(),
            style: font.style.clone(),
            weight: font.weight,
            slant: font.slant,
            source_name: font.source_name.clone(),
            subset_allowed: font.subset_allowed,
            content_hash: font.content_hash.clone(),
            path: font.path.clone(),
        }
    }
}

fn sanitize_layer(layer: &mut Layer) -> Result<()> {
    require_finite("layer opacity", layer.opacity)?;
    validate_transform(layer.transform)?;
    validate_adjustments(&layer.adjustments)?;
    validate_mask(layer.mask)?;
    validate_layer_style(&layer.style)?;
    validate_shape_stroke(layer.stroke)?;
    validate_pixel_mask(layer)?;
    if let Some(mask) = &layer.vector_mask {
        mask.validate()?;
    }
    if let Some(fill) = &layer.shape_fill {
        validate_shape_fill(fill)?;
        if !matches!(
            layer.kind,
            LayerKind::Rectangle { .. } | LayerKind::Ellipse { .. } | LayerKind::Path { .. }
        ) {
            bail!("only shape layers can have a shape fill");
        }
        if matches!(&layer.kind, LayerKind::Path { geometry, .. } if !geometry.closed()) {
            bail!("open path layers cannot have a shape fill");
        }
    }
    layer.opacity = layer.opacity.clamp(0.0, 1.0);
    layer.transform = layer.transform.sanitized();
    layer.adjustments = layer.adjustments.clone().sanitized();
    layer.mask = layer.mask.sanitized();
    layer.style = layer.style.clone().sanitized();
    layer.shape_fill = layer.shape_fill.clone().map(crate::ShapeFill::sanitized);
    layer.stroke = layer.stroke.sanitized();

    match &mut layer.kind {
        LayerKind::Raster { path, .. } => {
            *path = fs::canonicalize(&*path)
                .with_context(|| format!("could not open transferred raster {}", path.display()))?;
            image::ImageReader::open(&*path)
                .with_context(|| format!("could not open {}", path.display()))?
                .with_guessed_format()?
                .into_dimensions()
                .with_context(|| format!("could not inspect {}", path.display()))?;
        }
        LayerKind::Text {
            text,
            font_size,
            typography,
            ..
        } => {
            if text.trim().is_empty() {
                bail!("text cannot be empty");
            }
            require_finite("font size", *font_size)?;
            require_finite("line height", typography.line_height)?;
            require_finite("tracking", typography.tracking)?;
            require_finite("outline width", typography.effects.outline_width)?;
            require_finite("shadow x", typography.effects.shadow_offset_x)?;
            require_finite("shadow y", typography.effects.shadow_offset_y)?;
            if let Some(width) = typography.box_width {
                require_finite("text box width", width)?;
            }
            *font_size = (*font_size).clamp(4.0, 1_000.0);
            *typography = typography.clone().sanitized();
        }
        LayerKind::Rectangle {
            width,
            height,
            corner_radius,
            ..
        } => {
            require_finite("corner radius", *corner_radius)?;
            *width = (*width).clamp(1, MAX_CANVAS_DIMENSION);
            *height = (*height).clamp(1, MAX_CANVAS_DIMENSION);
            *corner_radius = (*corner_radius).max(0.0);
        }
        LayerKind::Ellipse { width, height, .. } => {
            *width = (*width).clamp(1, MAX_CANVAS_DIMENSION);
            *height = (*height).clamp(1, MAX_CANVAS_DIMENSION);
        }
        LayerKind::Path { .. } => {}
    }
    Ok(())
}

fn validate_pixel_mask(layer: &Layer) -> Result<()> {
    let Some(mask) = &layer.pixel_mask else {
        return Ok(());
    };
    if matches!(layer.kind, LayerKind::Path { .. }) {
        bail!("path layers cannot carry pixel masks; use a vector mask instead");
    }
    let pixels = u64::from(mask.width) * u64::from(mask.height);
    if pixels > crate::MAX_COLOR_SELECTION_PIXELS {
        bail!("transferred pixel mask exceeds the bounded pixel limit");
    }
    let expected =
        usize::try_from(pixels).context("pixel mask dimensions exceed platform limits")?;
    if mask.width == 0 || mask.height == 0 || mask.alpha.len() != expected {
        bail!("transferred layer has an invalid pixel mask");
    }
    if !mask.has_valid_identity() {
        bail!("transferred pixel mask identity does not match its alpha");
    }
    let dimensions =
        crate::shape_dimensions(layer).context("only shape layers can carry a pixel mask")?;
    if dimensions != (mask.width, mask.height) {
        bail!("transferred pixel mask dimensions do not match its shape");
    }
    Ok(())
}

fn import_transferred_font(document: &mut Document, font: LayerTransferFont) -> Result<u64> {
    let parsed = FontAsset::import(0, &font.path)?;
    if parsed.family != font.family
        || parsed.style != font.style
        || parsed.weight != font.weight
        || parsed.slant != font.slant
        || parsed.subset_allowed != font.subset_allowed
        || parsed.content_hash != font.content_hash
    {
        bail!("transferred font metadata does not match its OpenType bytes");
    }
    if let Some(existing) = document
        .font_assets
        .iter()
        .find(|existing| existing.content_hash == parsed.content_hash)
    {
        return Ok(existing.id);
    }

    let id = document.next_font_id;
    document.next_font_id += 1;
    document.font_assets.push(FontAsset {
        id,
        family: parsed.family,
        style: parsed.style,
        weight: parsed.weight,
        slant: parsed.slant,
        source_name: font.source_name,
        embedding_permission: parsed.embedding_permission,
        subset_allowed: parsed.subset_allowed,
        content_hash: parsed.content_hash,
        path: parsed.path,
        original_path: None,
    });
    Ok(id)
}
