//! Prism's command-driven layered document engine.

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use spectrum_imaging::Adjustments;

mod commands;
pub use commands::Command;

mod text;
use text::default_text_layer_name;

mod typography;
pub use typography::{FontAsset, FontSlant, TextAlignment, TextEffects, TextTypography};

mod transfer;
pub use transfer::{
    LAYER_TRANSFER_FORMAT, LAYER_TRANSFER_VERSION, LayerTransfer, LayerTransferFont,
};

mod validation;
use validation::*;

mod revisions;
pub use revisions::{DurableProject, ProjectHistory};

mod workspace;
pub use workspace::Workspace;

mod shapes;
pub use shapes::{
    RasterizedShapeAsset, interactive_shape_scale, rasterize_shape_asset,
    recommended_rasterization_scale, shape_dimensions,
};

pub const PRISM_VERSION: u32 = 2;
pub const MAX_HISTORY: usize = 100;
pub const MAX_CANVAS_DIMENSION: u32 = 16_384;

mod blend;
pub use blend::{BlendMode, blend_rgb};

mod alignment;
pub use alignment::{
    Alignment, AlignmentReference, Guide, GuideOrientation, LayerGeometry, align_layer_transform,
    layer_geometry, layer_geometry_with_bounds, layer_geometry_with_size,
};

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

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ShapeStroke {
    pub enabled: bool,
    pub width: f32,
    pub color: [u8; 4],
}

impl Default for ShapeStroke {
    fn default() -> Self {
        Self {
            enabled: false,
            width: 4.0,
            color: [255, 255, 255, 255],
        }
    }
}

impl ShapeStroke {
    fn sanitized(self) -> Self {
        Self {
            enabled: self.enabled,
            width: self.width.clamp(0.5, 512.0),
            color: self.color,
        }
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
    pub transform: Transform,
    pub adjustments: Adjustments,
    pub mask: LayerMask,
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
            transform: Transform::default(),
            adjustments: Adjustments::default(),
            mask: LayerMask::default(),
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

    fn allocate_id(&mut self) -> u64 {
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
            validate_shape_stroke(layer.stroke)?;
            validate_adjustments(&layer.adjustments)?;
            layer.opacity = layer.opacity.clamp(0.0, 1.0);
            layer.transform = layer.transform.sanitized();
            layer.mask = layer.mask.sanitized();
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
        }
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

#[derive(Clone, Debug, Serialize)]
pub struct CommandOutput {
    pub action: String,
    pub message: String,
    pub layer_ids: Vec<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub guide_ids: Vec<u64>,
}

fn apply_command(document: &mut Document, command: Command) -> Result<CommandOutput> {
    match command {
        Command::SetCanvas {
            width,
            height,
            background,
        } => {
            document.width = width.clamp(1, MAX_CANVAS_DIMENSION);
            document.height = height.clamp(1, MAX_CANVAS_DIMENSION);
            document.background = background;
            alignment::clamp_guides(document);
            Ok(output("set_canvas", "updated canvas", Vec::new()))
        }
        Command::CropCanvas {
            x,
            y,
            width,
            height,
        } => {
            if width == 0 || height == 0 || x >= document.width || y >= document.height {
                bail!("crop must overlap the canvas and have a nonzero size");
            }
            document.width = width.min(document.width - x);
            document.height = height.min(document.height - y);
            for layer in &mut document.layers {
                layer.transform.x -= x as f32;
                layer.transform.y -= y as f32;
            }
            alignment::crop_guides(document, x, y);
            Ok(output("crop_canvas", "cropped canvas", Vec::new()))
        }
        Command::AddRaster { path, name, x, y } => {
            require_finite("x", x)?;
            require_finite("y", y)?;
            let path = fs::canonicalize(&path)
                .with_context(|| format!("could not open raster layer {}", path.display()))?;
            image::ImageReader::open(&path)
                .with_context(|| format!("could not open {}", path.display()))?
                .with_guessed_format()?
                .into_dimensions()
                .with_context(|| format!("could not inspect {}", path.display()))?;
            let id = document.allocate_id();
            let layer_name = name.unwrap_or_else(|| {
                path.file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned()
            });
            document.layers.push(Layer {
                id,
                name: layer_name,
                transform: Transform {
                    x,
                    y,
                    ..Default::default()
                },
                kind: LayerKind::Raster {
                    original_path: Some(path.clone()),
                    path,
                },
                ..Default::default()
            });
            document.selected = Some(id);
            Ok(output("add_raster", "added raster layer", vec![id]))
        }
        Command::AddText {
            text,
            name,
            font_size,
            color,
            x,
            y,
        } => {
            require_finite("font size", font_size)?;
            require_finite("x", x)?;
            require_finite("y", y)?;
            if text.trim().is_empty() {
                bail!("text cannot be empty");
            }
            let id = document.allocate_id();
            document.layers.push(Layer {
                id,
                name: name.unwrap_or_else(|| default_text_layer_name(&text)),
                transform: Transform {
                    x,
                    y,
                    ..Default::default()
                },
                kind: LayerKind::Text {
                    text,
                    font_size: font_size.clamp(4.0, 1_000.0),
                    color,
                    typography: TextTypography::default(),
                },
                ..Default::default()
            });
            document.selected = Some(id);
            Ok(output("add_text", "added text layer", vec![id]))
        }
        Command::ImportFont { path } => {
            let id = document.next_font_id;
            let font = FontAsset::import(id, &path)?;
            if let Some(existing) = document
                .font_assets
                .iter()
                .find(|existing| existing.content_hash == font.content_hash)
            {
                return Ok(output(
                    "import_font",
                    &format!("font already embedded as asset {}", existing.id),
                    Vec::new(),
                ));
            }
            document.next_font_id += 1;
            let message = format!("embedded {} {} as font asset {id}", font.family, font.style);
            document.font_assets.push(font);
            Ok(output("import_font", &message, Vec::new()))
        }
        Command::AddRectangle {
            name,
            width,
            height,
            color,
            corner_radius,
            x,
            y,
        } => {
            require_finite("corner radius", corner_radius)?;
            require_finite("x", x)?;
            require_finite("y", y)?;
            let id = document.allocate_id();
            document.layers.push(Layer {
                id,
                name: name.unwrap_or_else(|| "Rectangle".into()),
                transform: Transform {
                    x,
                    y,
                    ..Default::default()
                },
                kind: LayerKind::Rectangle {
                    width: width.clamp(1, MAX_CANVAS_DIMENSION),
                    height: height.clamp(1, MAX_CANVAS_DIMENSION),
                    color,
                    corner_radius: corner_radius.max(0.0),
                },
                ..Default::default()
            });
            document.selected = Some(id);
            Ok(output("add_rectangle", "added rectangle layer", vec![id]))
        }
        Command::AddEllipse {
            name,
            width,
            height,
            color,
            x,
            y,
        } => {
            require_finite("x", x)?;
            require_finite("y", y)?;
            let id = document.allocate_id();
            document.layers.push(Layer {
                id,
                name: name.unwrap_or_else(|| "Ellipse".into()),
                transform: Transform {
                    x,
                    y,
                    ..Default::default()
                },
                kind: LayerKind::Ellipse {
                    width: width.clamp(1, MAX_CANVAS_DIMENSION),
                    height: height.clamp(1, MAX_CANVAS_DIMENSION),
                    color,
                },
                ..Default::default()
            });
            document.selected = Some(id);
            Ok(output("add_ellipse", "added ellipse layer", vec![id]))
        }
        Command::UpdateText {
            id,
            text,
            font_size,
            color,
        } => {
            require_finite("font size", font_size)?;
            if text.trim().is_empty() {
                bail!("text cannot be empty");
            }
            let layer = document.layer_mut(id)?;
            let auto_named = if let LayerKind::Text { text, .. } = &layer.kind {
                layer.name == default_text_layer_name(text)
            } else {
                bail!("layer {id} is not a text layer");
            };
            {
                let LayerKind::Text {
                    text: layer_text,
                    font_size: layer_size,
                    color: layer_color,
                    ..
                } = &mut layer.kind
                else {
                    unreachable!("text kind was checked above");
                };
                *layer_text = text;
                *layer_size = font_size.clamp(4.0, 1_000.0);
                *layer_color = color;
            }
            if auto_named && let LayerKind::Text { text, .. } = &layer.kind {
                layer.name = default_text_layer_name(text);
            }
            Ok(output("update_text", "updated text layer", vec![id]))
        }
        Command::SetTextTypography { id, typography } => {
            require_finite("line height", typography.line_height)?;
            require_finite("tracking", typography.tracking)?;
            require_finite("outline width", typography.effects.outline_width)?;
            require_finite("shadow x", typography.effects.shadow_offset_x)?;
            require_finite("shadow y", typography.effects.shadow_offset_y)?;
            if let Some(width) = typography.box_width {
                require_finite("text box width", width)?;
            }
            if let Some(font_id) = typography.font_id {
                document.font_asset(font_id)?;
            }
            let layer = document.layer_mut(id)?;
            let LayerKind::Text {
                typography: layer_typography,
                ..
            } = &mut layer.kind
            else {
                bail!("layer {id} is not a text layer");
            };
            *layer_typography = typography.sanitized();
            Ok(output(
                "set_text_typography",
                "updated typography",
                vec![id],
            ))
        }
        Command::UpdateRectangle {
            id,
            width,
            height,
            color,
            corner_radius,
        } => {
            require_finite("corner radius", corner_radius)?;
            let layer = document.layer_mut(id)?;
            let LayerKind::Rectangle {
                width: layer_width,
                height: layer_height,
                color: layer_color,
                corner_radius: layer_radius,
            } = &mut layer.kind
            else {
                bail!("layer {id} is not a rectangle layer");
            };
            *layer_width = width.clamp(1, MAX_CANVAS_DIMENSION);
            *layer_height = height.clamp(1, MAX_CANVAS_DIMENSION);
            *layer_color = color;
            *layer_radius = corner_radius.max(0.0);
            Ok(output(
                "update_rectangle",
                "updated rectangle layer",
                vec![id],
            ))
        }
        Command::UpdateEllipse {
            id,
            width,
            height,
            color,
        } => {
            let layer = document.layer_mut(id)?;
            let LayerKind::Ellipse {
                width: layer_width,
                height: layer_height,
                color: layer_color,
            } = &mut layer.kind
            else {
                bail!("layer {id} is not an ellipse layer");
            };
            *layer_width = width.clamp(1, MAX_CANVAS_DIMENSION);
            *layer_height = height.clamp(1, MAX_CANVAS_DIMENSION);
            *layer_color = color;
            Ok(output("update_ellipse", "updated ellipse layer", vec![id]))
        }
        Command::RemoveLayer { id } => {
            let index = document
                .layers
                .iter()
                .position(|layer| layer.id == id)
                .with_context(|| format!("layer {id} is not in this document"))?;
            document.layers.remove(index);
            if document.selected == Some(id) {
                document.selected = document.layers.last().map(|layer| layer.id);
            }
            Ok(output("remove_layer", "removed layer", vec![id]))
        }
        Command::DuplicateLayer { id } => {
            let mut layer = document.layer(id)?.clone();
            let new_id = document.allocate_id();
            layer.id = new_id;
            layer.name = format!("{} copy", layer.name);
            layer.transform.x += 16.0;
            layer.transform.y += 16.0;
            let index = document
                .layers
                .iter()
                .position(|layer| layer.id == id)
                .unwrap_or(document.layers.len());
            document.layers.insert(index + 1, layer);
            document.selected = Some(new_id);
            Ok(output("duplicate_layer", "duplicated layer", vec![new_id]))
        }
        Command::InsertLayer { transfer, index } => {
            let id = (*transfer).insert_into(document, index)?;
            Ok(output(
                "insert_layer",
                "inserted transferred layer",
                vec![id],
            ))
        }
        Command::RenameLayer { id, name } => {
            let name = name.trim();
            if name.is_empty() {
                bail!("layer name cannot be empty");
            }
            document.layer_mut(id)?.name = name.into();
            Ok(output("rename_layer", "renamed layer", vec![id]))
        }
        Command::SelectLayer { id } => {
            if let Some(id) = id {
                document.layer(id)?;
            }
            document.selected = id;
            Ok(output(
                "select_layer",
                "selected layer",
                id.into_iter().collect(),
            ))
        }
        Command::MoveLayer { id, index } => {
            let current = document
                .layers
                .iter()
                .position(|layer| layer.id == id)
                .with_context(|| format!("layer {id} is not in this document"))?;
            let layer = document.layers.remove(current);
            let index = index.min(document.layers.len());
            document.layers.insert(index, layer);
            Ok(output("move_layer", "reordered layer", vec![id]))
        }
        Command::SetVisibility { id, visible } => {
            document.layer_mut(id)?.visible = visible;
            Ok(output("set_visibility", "updated visibility", vec![id]))
        }
        Command::SetLocked { id, locked } => {
            document.layer_mut(id)?.locked = locked;
            Ok(output("set_locked", "updated layer lock", vec![id]))
        }
        Command::SetOpacity { id, opacity } => {
            require_finite("opacity", opacity)?;
            document.layer_mut(id)?.opacity = opacity.clamp(0.0, 1.0);
            Ok(output("set_opacity", "updated opacity", vec![id]))
        }
        Command::SetBlendMode { id, blend_mode } => {
            document.layer_mut(id)?.blend_mode = blend_mode;
            Ok(output("set_blend_mode", "updated blend mode", vec![id]))
        }
        Command::SetTransform { id, transform } => {
            validate_transform(transform)?;
            let layer = document.layer_mut(id)?;
            if layer.locked {
                bail!("layer {id} is locked");
            }
            layer.transform = transform.sanitized();
            Ok(output("set_transform", "transformed layer", vec![id]))
        }
        Command::SetRotation { id, degrees } => {
            require_finite("rotation", degrees)?;
            let layer = document.layer_mut(id)?;
            if layer.locked {
                bail!("layer {id} is locked");
            }
            layer.transform.rotation = degrees.rem_euclid(360.0);
            Ok(output("set_rotation", "rotated layer", vec![id]))
        }
        Command::AlignLayer {
            id,
            alignment,
            reference,
        } => {
            let transform = align_layer_transform(document, id, alignment, reference)?;
            document.layer_mut(id)?.transform = transform.sanitized();
            Ok(output("align_layer", "aligned layer", vec![id]))
        }
        Command::SetSnapping { enabled } => {
            document.snapping_enabled = enabled;
            Ok(output(
                "set_snapping",
                if enabled {
                    "enabled snapping"
                } else {
                    "disabled snapping"
                },
                Vec::new(),
            ))
        }
        Command::AddGuide {
            orientation,
            position,
        } => {
            let id = alignment::add_guide(document, orientation, position)?;
            Ok(guide_output("add_guide", "added guide", vec![id]))
        }
        Command::MoveGuide { id, position } => {
            alignment::move_guide(document, id, position)?;
            Ok(guide_output("move_guide", "moved guide", vec![id]))
        }
        Command::RemoveGuide { id } => {
            alignment::remove_guide(document, id)?;
            Ok(guide_output("remove_guide", "removed guide", vec![id]))
        }
        Command::AdjustLayer { id, patch } => {
            let layer = document.layer_mut(id)?;
            let mut adjustments = layer.adjustments.clone();
            patch.apply_to(&mut adjustments);
            validate_adjustments(&adjustments)?;
            layer.adjustments = adjustments;
            Ok(output("adjust_layer", "adjusted layer", vec![id]))
        }
        Command::ResetLayerAdjustments { id } => {
            document.layer_mut(id)?.adjustments = Adjustments::default();
            Ok(output(
                "reset_layer_adjustments",
                "reset layer adjustments",
                vec![id],
            ))
        }
        Command::SetMask { id, mask } => {
            validate_mask(mask)?;
            document.layer_mut(id)?.mask = mask.sanitized();
            Ok(output("set_mask", "updated layer mask", vec![id]))
        }
        Command::SetShapeStroke { id, stroke } => {
            validate_shape_stroke(stroke)?;
            let layer = document.layer_mut(id)?;
            if !matches!(
                layer.kind,
                LayerKind::Rectangle { .. } | LayerKind::Ellipse { .. }
            ) {
                bail!("layer {id} is not a shape layer");
            }
            layer.stroke = stroke.sanitized();
            Ok(output("set_shape_stroke", "updated shape stroke", vec![id]))
        }
        Command::RasterizeShape { id, path, scale } => {
            require_finite("rasterization scale", scale)?;
            if scale <= 0.0 {
                bail!("rasterization scale must be positive");
            }
            let (shape_width, shape_height) =
                shape_dimensions(document.layer(id)?).context("layer is not a parametric shape")?;
            let path = fs::canonicalize(&path)
                .with_context(|| format!("could not open rasterized shape {}", path.display()))?;
            let (width, height) = image::image_dimensions(&path).with_context(|| {
                format!("could not inspect rasterized shape {}", path.display())
            })?;
            let expected_width = (shape_width as f32 * scale).round().max(1.0) as u32;
            let expected_height = (shape_height as f32 * scale).round().max(1.0) as u32;
            if (width, height) != (expected_width, expected_height) {
                bail!(
                    "rasterized shape is {width}x{height}, expected {expected_width}x{expected_height} at {scale}x"
                );
            }
            let layer = document.layer_mut(id)?;
            layer.kind = LayerKind::Raster {
                path,
                original_path: None,
            };
            layer.transform.scale_x /= scale;
            layer.transform.scale_y /= scale;
            layer.stroke = ShapeStroke::default();
            Ok(output(
                "rasterize_shape",
                "rasterized shape layer",
                vec![id],
            ))
        }
        Command::SetClipping { id, enabled } => {
            document.layer_mut(id)?.clip_to_below = enabled;
            Ok(output("set_clipping", "updated clipping", vec![id]))
        }
        Command::Undo | Command::Redo => unreachable!("history is handled by Workspace"),
    }
}

fn output(action: &str, message: &str, layer_ids: Vec<u64>) -> CommandOutput {
    CommandOutput {
        action: action.into(),
        message: message.into(),
        layer_ids,
        guide_ids: Vec::new(),
    }
}

fn guide_output(action: &str, message: &str, guide_ids: Vec<u64>) -> CommandOutput {
    CommandOutput {
        action: action.into(),
        message: message.into(),
        layer_ids: Vec::new(),
        guide_ids,
    }
}

mod render;
mod render_region;
mod text_render;
mod text_rotation;

pub use render::{
    RegionRenderStats, RenderRegion, document_supports_region_native_zoom, export_document,
    load_document, render_document, render_document_region_scaled,
    render_document_region_scaled_with_stats, render_document_scaled, render_document_thumbnail,
    render_layer_base, render_layer_base_scaled, render_layer_base_scaled_with_font,
    render_layer_preview, render_layer_preview_scaled, render_layer_preview_scaled_with_font,
    render_solid_color, save_document,
};
pub use text_render::{
    TextGeometry, measure_text, measure_text_geometry, measure_text_geometry_with_typography,
    measure_text_with_typography,
};

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
