//! Prism's command-driven layered document engine.

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use spectrum_imaging::Adjustments;
use std::sync::Arc;

mod commands;
pub use commands::{Command, CommandOutput, PaintSelection};

mod effects;
mod effects_render;
pub use effects::{
    DROP_SHADOW_KERNEL, DROP_SHADOW_KERNEL_TOTAL_WEIGHT, DropShadow, GradientKind, GradientStop,
    LayerStyle, MAX_DROP_SHADOW_BLUR, MAX_DROP_SHADOW_OFFSET, ShapeFill, ShapeGradient,
    ShapeStroke,
};

mod text;

mod typography;
pub use typography::{
    FontAsset, FontEmbeddingPermission, FontSlant, TextAlignment, TextEffects, TextTypography,
};

mod bundled_font;
pub use bundled_font::{
    BUNDLED_FONT, BundledFontProvenance, LEGACY_BUNDLED_FONT_ALIAS, bundled_font_provenance,
    font_metadata_matches_query, is_bundled_font_family,
};

mod font_source;
pub use font_source::{FontSourceSnapshot, VerifiedFontSource};

mod font_usage;
pub use font_usage::{
    FontUsage, FontUsageAnalysis, UnicodeVariationSequence, analyze_all_font_usage,
    analyze_font_usage, font_usage,
};

mod font_subset_plan;
pub use font_subset_plan::{
    FontShapingSample, FontSubsetCandidate, FontSubsetPlan, plan_font_subset,
    plan_font_subset_with_verified_source,
};

mod transfer;
pub use transfer::{
    DISSOLVE_LAYER_TRANSFER_VERSION, LAYER_TRANSFER_FORMAT, LAYER_TRANSFER_VERSION, LayerTransfer,
    LayerTransferFont, PAINT_LAYER_TRANSFER_VERSION, PATH_LAYER_TRANSFER_VERSION,
};

mod validation;
use validation::*;

mod revisions;
pub use revisions::{
    DurableProject, ProjectHistory, ReadOnlyFontSource, ReadOnlyFontSubsetInput,
    inspect_font_source_read_only, inspect_font_subset_read_only,
};

mod workspace;
pub use workspace::Workspace;

mod shapes;
pub use shapes::{
    RasterizedShapeAsset, interactive_shape_scale, rasterize_shape_asset,
    recommended_rasterization_scale, shape_dimensions,
};

mod paths;
pub use paths::{
    MAX_PATH_ANCHORS, PATH_GEOMETRY_VERSION, PathAnchor, PathFillRule, PathGeometry,
    PathSourceBounds, VectorMask, apply_vector_mask_to_image, path_source_bounds,
};

mod paint;
mod paint_selection;
pub use paint::{
    BRUSH_PROGRAM_VERSION, BrushClip, BrushMode, BrushProgram, BrushSample, BrushStroke,
    BrushStyle, MAX_BRUSH_CLIP_BYTES_PER_PROGRAM, MAX_BRUSH_DABS_PER_PROGRAM,
    MAX_BRUSH_DABS_PER_STROKE, MAX_BRUSH_SAMPLES_PER_DOCUMENT, MAX_BRUSH_SAMPLES_PER_STROKE,
    MAX_BRUSH_STROKES_PER_LAYER, MAX_PAINT_REGION_PIXELS,
};

mod lasso;
pub use lasso::{
    LASSO_SUBPIXEL_SCALE, LassoPath, LassoPoint, MAX_LASSO_INPUT_POINTS,
    MAX_LASSO_RASTER_EDGE_TESTS, MAX_LASSO_VERTICES, SelectionCombineMode, apply_lasso_selection,
    combine_selections, lasso_selection,
};

pub const PRISM_VERSION: u32 = 8;
pub const PRISM_COMMAND_OPERATIONS_VERSION: u32 = 11;
pub const MAX_HISTORY: usize = 100;
pub const MAX_CANVAS_DIMENSION: u32 = 16_384;
pub const MAX_INLINE_PIXEL_MASK_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_DOCUMENT_NAME_CHARS: usize = 128;

mod blend;
pub use blend::{
    BlendMode, GroupCompositing, blend_rgb, dissolve_coverage, dissolve_pixel_present,
};

mod alignment;
pub use alignment::{
    Alignment, AlignmentReference, Guide, GuideOrientation, LayerGeometry, align_layer_transform,
    layer_geometry, layer_geometry_with_bounds, layer_geometry_with_size,
};

mod selection;
pub use selection::{MAX_COLOR_SELECTION_PIXELS, Selection, magic_wand_selection};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Transform {
    pub x: f32,
    pub y: f32,
    pub scale_x: f32,
    pub scale_y: f32,
    pub rotation: f32,
}

impl Default for Transform {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            scale_x: 1.0,
            scale_y: 1.0,
            rotation: 0.0,
        }
    }
}

impl Transform {
    fn sanitized(self) -> Self {
        Self {
            x: self.x.clamp(-100_000.0, 100_000.0),
            y: self.y.clamp(-100_000.0, 100_000.0),
            scale_x: self.scale_x.clamp(0.01, 100.0),
            scale_y: self.scale_y.clamp(0.01, 100.0),
            rotation: self.rotation.rem_euclid(360.0),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LayerMask {
    pub enabled: bool,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub invert: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PixelMask {
    pub width: u32,
    pub height: u32,
    pub content_hash: [u8; 32],
    #[serde(with = "pixel_mask_bytes")]
    pub alpha: Arc<[u8]>,
}

impl PixelMask {
    pub fn new(width: u32, height: u32, alpha: impl Into<Arc<[u8]>>) -> Self {
        let alpha = alpha.into();
        let content_hash = Sha256::digest(alpha.as_ref()).into();
        Self {
            width,
            height,
            content_hash,
            alpha,
        }
    }

    pub fn identity(&self) -> [u8; 32] {
        self.content_hash
    }

    pub(crate) fn has_valid_identity(&self) -> bool {
        self.content_hash == <[u8; 32]>::from(Sha256::digest(self.alpha.as_ref()))
    }
}

impl PartialEq for PixelMask {
    fn eq(&self, other: &Self) -> bool {
        self.width == other.width
            && self.height == other.height
            && self.content_hash == other.content_hash
            && (Arc::ptr_eq(&self.alpha, &other.alpha)
                || self.alpha.as_ref() == other.alpha.as_ref())
    }
}

impl Eq for PixelMask {}

mod pixel_mask_bytes {
    use base64::{Engine, engine::general_purpose::STANDARD};
    use serde::{Deserialize, Deserializer, Serializer};

    const MAX_ENCODED_BYTES: usize = (crate::MAX_COLOR_SELECTION_PIXELS as usize).div_ceil(3) * 4;

    pub fn serialize<S: Serializer>(
        bytes: &std::sync::Arc<[u8]>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&STANDARD.encode(bytes.as_ref()))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<std::sync::Arc<[u8]>, D::Error> {
        let encoded = String::deserialize(deserializer)?;
        if encoded.len() > MAX_ENCODED_BYTES {
            return Err(serde::de::Error::custom(
                "pixel mask exceeds the encoded size limit",
            ));
        }
        STANDARD
            .decode(encoded)
            .map(std::sync::Arc::from)
            .map_err(serde::de::Error::custom)
    }
}

impl Default for LayerMask {
    fn default() -> Self {
        Self {
            enabled: false,
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
            invert: false,
        }
    }
}

impl LayerMask {
    fn sanitized(self) -> Self {
        let x = self.x.clamp(0.0, 1.0);
        let y = self.y.clamp(0.0, 1.0);
        Self {
            enabled: self.enabled,
            x,
            y,
            width: self.width.clamp(0.001, 1.0 - x),
            height: self.height.clamp(0.001, 1.0 - y),
            invert: self.invert,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LayerKind {
    Raster {
        path: PathBuf,
        #[serde(default)]
        original_path: Option<PathBuf>,
    },
    Text {
        text: String,
        font_size: f32,
        color: [u8; 4],
        #[serde(default)]
        typography: TextTypography,
    },
    Rectangle {
        width: u32,
        height: u32,
        color: [u8; 4],
        corner_radius: f32,
    },
    Ellipse {
        width: u32,
        height: u32,
        color: [u8; 4],
    },
    Path {
        geometry: PathGeometry,
        color: [u8; 4],
    },
    Paint {
        program: BrushProgram,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Layer {
    pub id: u64,
    pub name: String,
    pub visible: bool,
    pub locked: bool,
    pub opacity: f32,
    pub blend_mode: BlendMode,
    pub dissolve_seed: u32,
    pub transform: Transform,
    pub adjustments: Adjustments,
    pub mask: LayerMask,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pixel_mask: Option<PixelMask>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vector_mask: Option<VectorMask>,
    #[serde(skip_serializing_if = "LayerStyle::is_empty")]
    pub style: LayerStyle,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shape_fill: Option<ShapeFill>,
    pub stroke: ShapeStroke,
    pub clip_to_below: bool,
    pub kind: LayerKind,
}

impl Default for Layer {
    fn default() -> Self {
        Self {
            id: 0,
            name: "Layer".into(),
            visible: true,
            locked: false,
            opacity: 1.0,
            blend_mode: BlendMode::Normal,
            dissolve_seed: 0,
            transform: Transform::default(),
            adjustments: Adjustments::default(),
            mask: LayerMask::default(),
            pixel_mask: None,
            vector_mask: None,
            style: LayerStyle::default(),
            shape_fill: None,
            stroke: ShapeStroke::default(),
            clip_to_below: false,
            kind: LayerKind::Rectangle {
                width: 100,
                height: 100,
                color: [255, 255, 255, 255],
                corner_radius: 0.0,
            },
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Document {
    pub version: u32,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub background: [u8; 4],
    pub guides: Vec<Guide>,
    pub snapping_enabled: bool,
    pub font_assets: Vec<FontAsset>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection: Option<Selection>,
    /// Bottom-to-top paint order.
    pub layers: Vec<Layer>,
    pub selected: Option<u64>,
    pub next_id: u64,
    pub next_guide_id: u64,
    pub next_font_id: u64,
}

impl Default for Document {
    fn default() -> Self {
        Self::new("Untitled", 1920, 1080)
    }
}

impl Document {
    pub fn new(name: impl Into<String>, width: u32, height: u32) -> Self {
        Self {
            version: PRISM_VERSION,
            name: name.into(),
            width: width.clamp(1, MAX_CANVAS_DIMENSION),
            height: height.clamp(1, MAX_CANVAS_DIMENSION),
            background: [24, 25, 29, 255],
            guides: Vec::new(),
            snapping_enabled: true,
            font_assets: Vec::new(),
            selection: None,
            layers: Vec::new(),
            selected: None,
            next_id: 1,
            next_guide_id: 1,
            next_font_id: 1,
        }
    }

    pub fn layer(&self, id: u64) -> Result<&Layer> {
        self.layers
            .iter()
            .find(|layer| layer.id == id)
            .with_context(|| format!("layer {id} is not in this document"))
    }

    pub fn layer_mut(&mut self, id: u64) -> Result<&mut Layer> {
        self.layers
            .iter_mut()
            .find(|layer| layer.id == id)
            .with_context(|| format!("layer {id} is not in this document"))
    }

    pub(crate) fn allocate_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    pub fn guide(&self, id: u64) -> Result<&Guide> {
        self.guides
            .iter()
            .find(|guide| guide.id == id)
            .with_context(|| format!("guide {id} is not in this document"))
    }

    pub fn font_asset(&self, id: u64) -> Result<&FontAsset> {
        self.font_assets
            .iter()
            .find(|font| font.id == id)
            .with_context(|| format!("font asset {id} is not in this document"))
    }

    pub fn font_for_layer(&self, layer: &Layer) -> Option<&FontAsset> {
        let LayerKind::Text { typography, .. } = &layer.kind else {
            return None;
        };
        typography.font_id.and_then(|id| self.font_asset(id).ok())
    }

    pub(crate) fn validate_inline_mask_budget(&self) -> Result<()> {
        self.validate_projected_inline_mask_budget(self.selection.as_ref(), 0)
    }

    pub(crate) fn validate_projected_inline_mask_budget(
        &self,
        selection: Option<&Selection>,
        additional_layer_bytes: usize,
    ) -> Result<()> {
        let selection_bytes = selection.and_then(Selection::alpha).map_or(0, <[u8]>::len);
        let layer_bytes = self
            .layers
            .iter()
            .try_fold(0_usize, |total, layer| {
                total
                    .checked_add(layer.pixel_mask.as_ref().map_or(0, |mask| mask.alpha.len()))
                    .and_then(|total| {
                        total.checked_add(match &layer.kind {
                            LayerKind::Paint { program } => program.clip_bytes(),
                            _ => 0,
                        })
                    })
            })
            .context("document inline pixel-mask byte count overflows")?;
        let inline_mask_bytes = selection_bytes
            .checked_add(layer_bytes)
            .and_then(|total| total.checked_add(additional_layer_bytes))
            .context("document inline pixel-mask byte count overflows")?;
        if inline_mask_bytes > MAX_INLINE_PIXEL_MASK_BYTES {
            bail!("document inline pixel masks exceed the 64 MiB aggregate limit");
        }
        Ok(())
    }

    pub(crate) fn validate_projected_paint_budget(
        &self,
        replacing: Option<&BrushProgram>,
        adding: &BrushProgram,
    ) -> Result<()> {
        let (samples, dabs) =
            self.layers
                .iter()
                .try_fold((0usize, 0usize), |(samples, dabs), layer| {
                    match &layer.kind {
                        LayerKind::Paint { program } => Ok::<(usize, usize), anyhow::Error>((
                            samples
                                .checked_add(program.sample_count())
                                .context("document Paint sample count overflows")?,
                            dabs.checked_add(program.dab_count()?)
                                .context("document Paint dab count overflows")?,
                        )),
                        _ => Ok((samples, dabs)),
                    }
                })?;
        let removed_samples = replacing.map_or(0, BrushProgram::sample_count);
        let removed_dabs = replacing
            .map(BrushProgram::dab_count)
            .transpose()?
            .unwrap_or(0);
        let adding_dabs = adding.dab_count()?;
        let projected_samples = samples
            .checked_sub(removed_samples)
            .and_then(|value| value.checked_add(adding.sample_count()))
            .context("document Paint sample count overflows")?;
        let projected_dabs = dabs
            .checked_sub(removed_dabs)
            .and_then(|value| value.checked_add(adding_dabs))
            .context("document Paint dab count overflows")?;
        if projected_samples > MAX_BRUSH_SAMPLES_PER_DOCUMENT {
            bail!("document exceeds the aggregate Paint sample limit");
        }
        if projected_dabs > MAX_BRUSH_DABS_PER_PROGRAM {
            bail!("document exceeds the aggregate Paint dab limit");
        }
        Ok(())
    }

    fn migrate(&mut self) -> Result<()> {
        if self.version > PRISM_VERSION {
            bail!(
                "project version {} is newer than this app supports ({PRISM_VERSION})",
                self.version
            );
        }
        self.version = PRISM_VERSION;
        self.width = self.width.clamp(1, MAX_CANVAS_DIMENSION);
        self.height = self.height.clamp(1, MAX_CANVAS_DIMENSION);
        for guide in &mut self.guides {
            guide.sanitize(self.width, self.height)?;
        }
        for layer in &mut self.layers {
            require_finite("layer opacity", layer.opacity)?;
            validate_transform(layer.transform)?;
            validate_mask(layer.mask)?;
            if let Some(mask) = &layer.pixel_mask {
                if matches!(layer.kind, LayerKind::Path { .. }) {
                    bail!("path layers cannot carry pixel masks; use a vector mask instead");
                }
                let pixels = u64::from(mask.width) * u64::from(mask.height);
                if pixels > MAX_COLOR_SELECTION_PIXELS {
                    bail!(
                        "layer {} pixel mask exceeds the bounded pixel limit",
                        layer.id
                    );
                }
                let expected = usize::try_from(pixels)
                    .context("pixel mask dimensions exceed platform limits")?;
                if mask.width == 0 || mask.height == 0 || mask.alpha.len() != expected {
                    bail!("layer {} has an invalid pixel mask", layer.id);
                }
                if !mask.has_valid_identity() {
                    bail!(
                        "layer {} pixel mask identity does not match its alpha",
                        layer.id
                    );
                }
                let dimensions = match &layer.kind {
                    LayerKind::Paint { program } => (program.width, program.height),
                    _ => shape_dimensions(layer)
                        .context("only shape and Paint layers can carry a pixel mask")?,
                };
                if dimensions != (mask.width, mask.height) {
                    bail!(
                        "layer {} pixel mask dimensions do not match its shape",
                        layer.id
                    );
                }
            }
            if let Some(mask) = &layer.vector_mask {
                mask.validate()?;
            }
            effects::validate_layer_style(&layer.style)?;
            validate_shape_stroke(layer.stroke)?;
            if let Some(fill) = &layer.shape_fill {
                effects::validate_shape_fill(fill)?;
                if !matches!(
                    layer.kind,
                    LayerKind::Rectangle { .. }
                        | LayerKind::Ellipse { .. }
                        | LayerKind::Path { .. }
                ) {
                    bail!("only shape layers can have a shape fill");
                }
                if matches!(&layer.kind, LayerKind::Path { geometry, .. } if !geometry.closed()) {
                    bail!("open path layers cannot have a shape fill");
                }
            }
            validate_adjustments(&layer.adjustments)?;
            layer.opacity = layer.opacity.clamp(0.0, 1.0);
            layer.transform = layer.transform.sanitized();
            layer.mask = layer.mask.sanitized();
            layer.style = layer.style.clone().sanitized();
            layer.shape_fill = layer.shape_fill.clone().map(ShapeFill::sanitized);
            layer.stroke = layer.stroke.sanitized();
            layer.adjustments = layer.adjustments.clone().sanitized();
            if let LayerKind::Text { typography, .. } = &mut layer.kind {
                *typography = typography.clone().sanitized();
                if typography
                    .font_id
                    .is_some_and(|id| !self.font_assets.iter().any(|font| font.id == id))
                {
                    typography.font_id = None;
                }
            }
            if let LayerKind::Paint { program } = &layer.kind {
                program.validate()?;
            }
        }
        let paint_samples = self
            .layers
            .iter()
            .try_fold(0_usize, |total, layer| {
                total.checked_add(match &layer.kind {
                    LayerKind::Paint { program } => program.sample_count(),
                    _ => 0,
                })
            })
            .context("document Paint sample count overflows")?;
        if paint_samples > MAX_BRUSH_SAMPLES_PER_DOCUMENT {
            bail!("document exceeds the aggregate Paint sample limit");
        }
        let paint_dabs = self.layers.iter().try_fold(0_usize, |total, layer| {
            total
                .checked_add(match &layer.kind {
                    LayerKind::Paint { program } => program.dab_count()?,
                    _ => 0,
                })
                .context("document Paint dab count overflows")
        })?;
        if paint_dabs > MAX_BRUSH_DABS_PER_PROGRAM {
            bail!("document exceeds the aggregate Paint dab limit");
        }
        self.selection = self
            .selection
            .take()
            .map(|selection| selection.validated(self.width, self.height))
            .transpose()?;
        self.validate_inline_mask_budget()?;
        self.next_id = self
            .next_id
            .max(self.layers.iter().map(|layer| layer.id).max().unwrap_or(0) + 1);
        self.next_guide_id = self
            .next_guide_id
            .max(self.guides.iter().map(|guide| guide.id).max().unwrap_or(0) + 1);
        self.next_font_id = self.next_font_id.max(
            self.font_assets
                .iter()
                .map(|font| font.id)
                .max()
                .unwrap_or(0)
                + 1,
        );
        if self.selected.is_some_and(|id| self.layer(id).is_err()) {
            self.selected = self.layers.last().map(|layer| layer.id);
        }
        Ok(())
    }
}

mod command_apply;
use command_apply::apply_command;
use commands::output;

mod raster_backing_cache;
mod raster_region;
mod raster_sources;
mod render;
mod render_fallback;
mod render_region;
mod text_preview_cache;
mod text_render;
mod text_rotation;
mod transform_math;

pub use transform_math::rotation_sin_cos;

pub use raster_backing_cache::{
    DerivedBackingCache, DerivedBackingIdentity, DerivedBackingLimits, DerivedBackingMemoryPlan,
    DerivedBackingReadError, DerivedRasterBacking, PrepareDerivedBacking,
};
pub use raster_region::{RasterRegionInspection, inspect_raster_region_source};
pub use raster_sources::{RasterSourceEpoch, RasterSourceResolver, ResolvedRasterSource};
pub use render::{
    RegionRenderStats, RenderRegion, document_supports_region_native_zoom,
    document_supports_region_native_zoom_with_sources, export_document, load_document,
    render_document, render_document_region_scaled, render_document_region_scaled_with_sources,
    render_document_region_scaled_with_sources_and_stats, render_document_region_scaled_with_stats,
    render_document_scaled, render_document_thumbnail, render_layer_base, render_layer_base_scaled,
    render_layer_base_scaled_with_font, render_layer_preview, render_layer_preview_scaled,
    render_layer_preview_scaled_with_font, render_solid_color, save_document,
};
pub use render_region::{RegionSourceScales, recommended_text_raster_scale, region_source_scales};
pub use text_preview_cache::{LayerPreviewSchedule, TextPreviewFrameCache};
pub use text_render::{
    TextGeometry, measure_text, measure_text_geometry, measure_text_geometry_with_typography,
    measure_text_with_typography,
};

/// Applies one Paint gesture to a detached document clone without history or revision I/O.
/// GUI drafts use this exact command path before committing the same command once on release.
pub fn preview_paint_command(document: &Document, command: Command) -> Result<Document> {
    if !matches!(
        command,
        Command::AddBrushStroke { .. } | Command::AddPaintLayerWithStroke { .. }
    ) {
        bail!("Paint preview accepts only completed Paint gesture commands");
    }
    let mut preview = document.clone();
    apply_command(&mut preview, command)?;
    Ok(preview)
}

/// Whether raw Paint coordinates still map directly through the layer's outer transform.
/// Geometric image adjustments require an adjusted-to-source inverse that v1 painting does not
/// yet expose, so interactive and command-driven strokes fail closed instead of landing silently
/// in the wrong source pixels.
pub fn paint_layer_allows_direct_strokes(layer: &Layer) -> bool {
    let adjustments = &layer.adjustments;
    adjustments.rotation.rem_euclid(360) == 0
        && !adjustments.flip_horizontal
        && !adjustments.flip_vertical
        && adjustments.straighten.abs() <= 0.01
        && adjustments.crop.is_none()
}

#[cfg(test)]
mod document_lifecycle_tests;
#[cfg(test)]
#[path = "core_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "shape_tests.rs"]
mod shape_tests;

#[cfg(test)]
#[path = "render_region_tests.rs"]
mod render_region_tests;

#[cfg(test)]
#[path = "render_fallback_tests.rs"]
mod render_fallback_tests;

#[cfg(test)]
#[path = "raster_backing_cache_tests.rs"]
mod raster_backing_cache_tests;

#[cfg(test)]
#[path = "raster_source_resolver_tests.rs"]
mod raster_source_resolver_tests;

#[cfg(test)]
#[path = "raster_backing_eviction_tests.rs"]
mod raster_backing_eviction_tests;

#[cfg(test)]
#[path = "rotation_tests.rs"]
mod rotation_tests;

#[cfg(test)]
#[path = "alignment_tests.rs"]
mod alignment_tests;

#[cfg(test)]
#[path = "typography_tests.rs"]
mod typography_tests;

#[cfg(test)]
#[path = "transfer_tests.rs"]
mod transfer_tests;

#[cfg(test)]
#[path = "workspace_interaction_tests.rs"]
mod workspace_interaction_tests;

#[cfg(test)]
#[path = "durable_asset_tests.rs"]
mod durable_asset_tests;

#[cfg(test)]
#[path = "effect_tests.rs"]
mod effect_tests;

#[cfg(test)]
#[path = "selection_tests.rs"]
mod selection_tests;

#[cfg(test)]
#[path = "path_tests.rs"]
mod path_tests;

#[cfg(test)]
#[path = "paint_limits_tests.rs"]
mod paint_limits_tests;
#[cfg(test)]
#[path = "paint_tests.rs"]
mod paint_tests;
