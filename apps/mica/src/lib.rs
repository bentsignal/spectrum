//! Mica's command-driven layered document engine.

use std::{
    fs,
    io::BufWriter,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use fontdue::Font;
use image::{DynamicImage, ImageEncoder, Rgba, RgbaImage, imageops::FilterType};
use lumen_core::{
    AdjustmentPatch, Adjustments,
    engine::{RenderOptions, render_image},
};
use serde::{Deserialize, Serialize};

pub const MICA_VERSION: u32 = 1;
pub const MAX_HISTORY: usize = 100;
pub const MAX_CANVAS_DIMENSION: u32 = 16_384;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlendMode {
    #[default]
    Normal,
    Multiply,
    Screen,
    Overlay,
}

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
    },
    Rectangle {
        width: u32,
        height: u32,
        color: [u8; 4],
        corner_radius: f32,
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
    /// Bottom-to-top paint order.
    pub layers: Vec<Layer>,
    pub selected: Option<u64>,
    pub next_id: u64,
}

impl Default for Document {
    fn default() -> Self {
        Self::new("Untitled", 1920, 1080)
    }
}

impl Document {
    pub fn new(name: impl Into<String>, width: u32, height: u32) -> Self {
        Self {
            version: MICA_VERSION,
            name: name.into(),
            width: width.clamp(1, MAX_CANVAS_DIMENSION),
            height: height.clamp(1, MAX_CANVAS_DIMENSION),
            background: [24, 25, 29, 255],
            layers: Vec::new(),
            selected: None,
            next_id: 1,
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

    fn migrate(&mut self) -> Result<()> {
        if self.version > MICA_VERSION {
            bail!(
                "project version {} is newer than this app supports ({MICA_VERSION})",
                self.version
            );
        }
        self.version = MICA_VERSION;
        self.width = self.width.clamp(1, MAX_CANVAS_DIMENSION);
        self.height = self.height.clamp(1, MAX_CANVAS_DIMENSION);
        for layer in &mut self.layers {
            require_finite("layer opacity", layer.opacity)?;
            validate_transform(layer.transform)?;
            validate_mask(layer.mask)?;
            validate_adjustments(&layer.adjustments)?;
            layer.opacity = layer.opacity.clamp(0.0, 1.0);
            layer.transform = layer.transform.sanitized();
            layer.mask = layer.mask.sanitized();
            layer.adjustments = layer.adjustments.clone().sanitized();
        }
        self.next_id = self
            .next_id
            .max(self.layers.iter().map(|layer| layer.id).max().unwrap_or(0) + 1);
        if self.selected.is_some_and(|id| self.layer(id).is_err()) {
            self.selected = self.layers.last().map(|layer| layer.id);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum Command {
    SetCanvas {
        width: u32,
        height: u32,
        background: [u8; 4],
    },
    CropCanvas {
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    },
    AddRaster {
        path: PathBuf,
        name: Option<String>,
        x: f32,
        y: f32,
    },
    AddText {
        text: String,
        name: Option<String>,
        font_size: f32,
        color: [u8; 4],
        x: f32,
        y: f32,
    },
    AddRectangle {
        name: Option<String>,
        width: u32,
        height: u32,
        color: [u8; 4],
        corner_radius: f32,
        x: f32,
        y: f32,
    },
    UpdateText {
        id: u64,
        text: String,
        font_size: f32,
        color: [u8; 4],
    },
    UpdateRectangle {
        id: u64,
        width: u32,
        height: u32,
        color: [u8; 4],
        corner_radius: f32,
    },
    RemoveLayer {
        id: u64,
    },
    DuplicateLayer {
        id: u64,
    },
    RenameLayer {
        id: u64,
        name: String,
    },
    SelectLayer {
        id: Option<u64>,
    },
    MoveLayer {
        id: u64,
        index: usize,
    },
    SetVisibility {
        id: u64,
        visible: bool,
    },
    SetLocked {
        id: u64,
        locked: bool,
    },
    SetOpacity {
        id: u64,
        opacity: f32,
    },
    SetBlendMode {
        id: u64,
        blend_mode: BlendMode,
    },
    SetTransform {
        id: u64,
        transform: Transform,
    },
    AdjustLayer {
        id: u64,
        patch: AdjustmentPatch,
    },
    ResetLayerAdjustments {
        id: u64,
    },
    SetMask {
        id: u64,
        mask: LayerMask,
    },
    SetClipping {
        id: u64,
        enabled: bool,
    },
    Undo,
    Redo,
}

#[derive(Clone, Debug, Serialize)]
pub struct CommandOutput {
    pub action: String,
    pub message: String,
    pub layer_ids: Vec<u64>,
}

pub struct Workspace {
    pub document: Document,
    pub project_path: Option<PathBuf>,
    undo: Vec<Document>,
    redo: Vec<Document>,
    dirty: bool,
}

impl Default for Workspace {
    fn default() -> Self {
        Self::new(Document::default(), None)
    }
}

impl Workspace {
    pub fn new(document: Document, project_path: Option<PathBuf>) -> Self {
        Self {
            document,
            project_path,
            undo: Vec::new(),
            redo: Vec::new(),
            dirty: false,
        }
    }

    pub fn open(path: &Path) -> Result<Self> {
        Ok(Self::new(load_document(path)?, Some(path.to_owned())))
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn save(&mut self, path: Option<&Path>) -> Result<PathBuf> {
        let path = path
            .map(Path::to_owned)
            .or_else(|| self.project_path.clone())
            .context("choose a .mica project path first")?;
        save_document(&self.document, &path)?;
        self.document = load_document(&path)?;
        self.project_path = Some(path.clone());
        self.dirty = false;
        Ok(path)
    }

    pub fn execute(&mut self, command: Command) -> Result<CommandOutput> {
        match command {
            Command::Undo => self.undo(),
            Command::Redo => self.redo(),
            command @ Command::SelectLayer { .. } => apply_command(&mut self.document, command),
            command => {
                let before = self.document.clone();
                let output = apply_command(&mut self.document, command)?;
                if self.document != before {
                    self.undo.push(before);
                    if self.undo.len() > MAX_HISTORY {
                        self.undo.remove(0);
                    }
                    self.redo.clear();
                    self.dirty = true;
                }
                Ok(output)
            }
        }
    }

    fn undo(&mut self) -> Result<CommandOutput> {
        let previous = self.undo.pop().context("nothing to undo")?;
        self.redo
            .push(std::mem::replace(&mut self.document, previous));
        self.dirty = true;
        Ok(output("undo", "went back one edit", Vec::new()))
    }

    fn redo(&mut self) -> Result<CommandOutput> {
        let next = self.redo.pop().context("nothing to redo")?;
        self.undo.push(std::mem::replace(&mut self.document, next));
        self.dirty = true;
        Ok(output("redo", "went forward one edit", Vec::new()))
    }
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
                name: name.unwrap_or_else(|| text.chars().take(24).collect()),
                transform: Transform {
                    x,
                    y,
                    ..Default::default()
                },
                kind: LayerKind::Text {
                    text,
                    font_size: font_size.clamp(4.0, 1_000.0),
                    color,
                },
                ..Default::default()
            });
            document.selected = Some(id);
            Ok(output("add_text", "added text layer", vec![id]))
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
            let LayerKind::Text {
                text: layer_text,
                font_size: layer_size,
                color: layer_color,
            } = &mut layer.kind
            else {
                bail!("layer {id} is not a text layer");
            };
            *layer_text = text;
            *layer_size = font_size.clamp(4.0, 1_000.0);
            *layer_color = color;
            Ok(output("update_text", "updated text layer", vec![id]))
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
    }
}

fn require_finite(label: &str, value: f32) -> Result<()> {
    if !value.is_finite() {
        bail!("{label} must be a finite number");
    }
    Ok(())
}

fn validate_transform(transform: Transform) -> Result<()> {
    for (label, value) in [
        ("x", transform.x),
        ("y", transform.y),
        ("horizontal scale", transform.scale_x),
        ("vertical scale", transform.scale_y),
        ("rotation", transform.rotation),
    ] {
        require_finite(label, value)?;
    }
    Ok(())
}

fn validate_mask(mask: LayerMask) -> Result<()> {
    for (label, value) in [
        ("mask x", mask.x),
        ("mask y", mask.y),
        ("mask width", mask.width),
        ("mask height", mask.height),
    ] {
        require_finite(label, value)?;
    }
    Ok(())
}

fn validate_adjustments(value: &Adjustments) -> Result<()> {
    for (label, number) in [
        ("exposure", value.exposure),
        ("temperature", value.temperature),
        ("tint", value.tint),
        ("contrast", value.contrast),
        ("highlights", value.highlights),
        ("shadows", value.shadows),
        ("whites", value.whites),
        ("blacks", value.blacks),
        ("texture", value.texture),
        ("clarity", value.clarity),
        ("dehaze", value.dehaze),
        ("vibrance", value.vibrance),
        ("saturation", value.saturation),
        ("vignette", value.vignette),
        ("sharpening", value.sharpening),
        ("noise reduction", value.noise_reduction),
        ("straighten", value.straighten),
        ("color balance", value.color_grading.balance),
    ] {
        require_finite(label, number)?;
    }
    if let Some(crop) = value.crop {
        for (label, number) in [
            ("crop x", crop.x),
            ("crop y", crop.y),
            ("crop width", crop.width),
            ("crop height", crop.height),
        ] {
            require_finite(label, number)?;
        }
    }
    for band in value.hsl.bands() {
        require_finite("HSL hue", band.hue)?;
        require_finite("HSL saturation", band.saturation)?;
        require_finite("HSL luminance", band.luminance)?;
    }
    for curve in [
        &value.curves.master,
        &value.curves.red,
        &value.curves.green,
        &value.curves.blue,
    ] {
        for point in &curve.points {
            require_finite("curve x", point.x)?;
            require_finite("curve y", point.y)?;
        }
    }
    for grade in [
        value.color_grading.shadows,
        value.color_grading.midtones,
        value.color_grading.highlights,
    ] {
        require_finite("grade hue", grade.hue)?;
        require_finite("grade saturation", grade.saturation)?;
        require_finite("grade luminance", grade.luminance)?;
    }
    for spot in &value.spots {
        require_finite("spot x", spot.x)?;
        require_finite("spot y", spot.y)?;
        require_finite("spot radius", spot.radius)?;
        require_finite("spot opacity", spot.opacity)?;
    }
    Ok(())
}

pub fn save_document(document: &Document, path: &Path) -> Result<()> {
    if path.extension().and_then(|value| value.to_str()) != Some("mica") {
        bail!("Mica projects must use the .mica extension");
    }
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)
            .with_context(|| format!("could not create {}", parent.display()))?;
    }
    let directory = fs::canonicalize(
        path.parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new(".")),
    )?;
    let project_stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("mica");
    let asset_directory = directory.join(format!("{project_stem}-assets"));
    let mut portable = document.clone();
    for layer in &mut portable.layers {
        if let LayerKind::Raster {
            path: source,
            original_path,
        } = &mut layer.kind
        {
            let canonical = fs::canonicalize(&*source)
                .with_context(|| format!("could not read layer source {}", source.display()))?;
            if original_path.is_none() {
                *original_path = Some(canonical.clone());
            }
            if let Ok(relative) = canonical.strip_prefix(&directory) {
                *source = relative.to_owned();
            } else {
                fs::create_dir_all(&asset_directory)?;
                let file_name = canonical
                    .file_name()
                    .and_then(|value| value.to_str())
                    .unwrap_or("image");
                let destination = asset_directory.join(format!("layer-{}-{file_name}", layer.id));
                fs::copy(&canonical, &destination).with_context(|| {
                    format!(
                        "could not copy {} into portable Mica assets",
                        canonical.display()
                    )
                })?;
                *source = destination.strip_prefix(&directory)?.to_owned();
            }
        }
    }
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".tmp");
    let temporary = PathBuf::from(temporary);
    fs::write(&temporary, serde_json::to_vec_pretty(&portable)?)
        .with_context(|| format!("could not write {}", temporary.display()))?;
    #[cfg(not(target_os = "windows"))]
    fs::rename(&temporary, path)
        .with_context(|| format!("could not replace {}", path.display()))?;
    #[cfg(target_os = "windows")]
    replace_file_windows_safe(&temporary, path)?;
    Ok(())
}

#[cfg(target_os = "windows")]
fn replace_file_windows_safe(temporary: &Path, destination: &Path) -> Result<()> {
    if !destination.exists() {
        fs::rename(temporary, destination)?;
        return Ok(());
    }
    let mut backup = destination.as_os_str().to_owned();
    backup.push(".backup");
    let backup = PathBuf::from(backup);
    if backup.exists() {
        fs::remove_file(&backup)?;
    }
    fs::rename(destination, &backup)?;
    match fs::rename(temporary, destination) {
        Ok(()) => {
            fs::remove_file(backup)?;
            Ok(())
        }
        Err(error) => {
            let _ = fs::rename(&backup, destination);
            Err(error).with_context(|| format!("could not replace {}", destination.display()))
        }
    }
}

pub fn load_document(path: &Path) -> Result<Document> {
    let bytes = fs::read(path).with_context(|| format!("could not read {}", path.display()))?;
    let mut document: Document = serde_json::from_slice(&bytes)
        .with_context(|| format!("invalid Mica project {}", path.display()))?;
    document.migrate()?;
    let directory = path.parent().unwrap_or_else(|| Path::new("."));
    for layer in &mut document.layers {
        if let LayerKind::Raster { path, .. } = &mut layer.kind
            && path.is_relative()
        {
            *path = directory.join(&*path);
            if let Ok(canonical) = fs::canonicalize(&*path) {
                *path = canonical;
            }
        }
    }
    Ok(document)
}

pub fn render_document(document: &Document, max_size: Option<u32>) -> Result<DynamicImage> {
    let longest = document.width.max(document.height) as f32;
    let scale = max_size
        .filter(|size| *size > 0)
        .map_or(1.0, |size| (size as f32 / longest).min(1.0));
    let canvas_width = (document.width as f32 * scale).round().max(1.0) as u32;
    let canvas_height = (document.height as f32 * scale).round().max(1.0) as u32;
    let mut canvas = RgbaImage::from_pixel(canvas_width, canvas_height, Rgba(document.background));
    let mut previous_coverage: Option<RgbaImage> = None;
    for layer in &document.layers {
        if !layer.visible || layer.opacity <= 0.0 {
            continue;
        }
        let source = render_layer_source(layer)?;
        let mut scaled_layer = layer.clone();
        scaled_layer.transform.x *= scale;
        scaled_layer.transform.y *= scale;
        scaled_layer.transform.scale_x *= scale;
        scaled_layer.transform.scale_y *= scale;
        let source = transform_layer(source, scaled_layer.transform);
        let mut coverage = RgbaImage::new(canvas_width, canvas_height);
        composite_layer(
            &mut canvas,
            &mut coverage,
            &source,
            &scaled_layer,
            previous_coverage.as_ref(),
        );
        previous_coverage = Some(coverage);
    }
    Ok(DynamicImage::ImageRgba8(canvas))
}

fn render_layer_source(layer: &Layer) -> Result<DynamicImage> {
    let image = match &layer.kind {
        LayerKind::Raster { path, .. } => image::ImageReader::open(path)
            .with_context(|| format!("could not open {}", path.display()))?
            .with_guessed_format()?
            .decode()
            .with_context(|| format!("could not decode {}", path.display()))?,
        LayerKind::Text {
            text,
            font_size,
            color,
        } => DynamicImage::ImageRgba8(render_text(text, *font_size, *color)?),
        LayerKind::Rectangle {
            width,
            height,
            color,
            corner_radius,
        } => DynamicImage::ImageRgba8(render_rectangle(*width, *height, *color, *corner_radius)),
    };
    Ok(render_image(
        image,
        layer.adjustments.clone(),
        RenderOptions::default(),
    ))
}

fn render_text(text: &str, font_size: f32, color: [u8; 4]) -> Result<RgbaImage> {
    let font = Font::from_bytes(
        epaint_default_fonts::UBUNTU_LIGHT,
        fontdue::FontSettings::default(),
    )
    .map_err(|error| anyhow::anyhow!("could not load bundled font: {error}"))?;
    let line_height = (font_size * 1.25).ceil() as u32;
    let lines: Vec<_> = text.lines().collect();
    let width = lines
        .iter()
        .map(|line| {
            line.chars()
                .map(|character| font.metrics(character, font_size).advance_width)
                .sum::<f32>()
                .ceil() as u32
        })
        .max()
        .unwrap_or(1)
        .max(1);
    let height = (line_height * lines.len().max(1) as u32).max(1);
    let mut output = RgbaImage::new(width, height);
    for (line_index, line) in lines.iter().enumerate() {
        let mut cursor_x: f32 = 0.0;
        for character in line.chars() {
            let (metrics, bitmap) = font.rasterize(character, font_size);
            let origin_y = line_index as i32 * line_height as i32
                + (line_height as i32 - metrics.height as i32)
                + metrics.ymin.min(0).abs();
            for row in 0..metrics.height {
                for column in 0..metrics.width {
                    let x = cursor_x.round() as i32 + metrics.xmin + column as i32;
                    let y = origin_y + row as i32;
                    if x >= 0 && y >= 0 && (x as u32) < width && (y as u32) < height {
                        let alpha =
                            bitmap[row * metrics.width + column] as u16 * color[3] as u16 / 255;
                        output.put_pixel(
                            x as u32,
                            y as u32,
                            Rgba([color[0], color[1], color[2], alpha as u8]),
                        );
                    }
                }
            }
            cursor_x += metrics.advance_width;
        }
    }
    Ok(output)
}

fn render_rectangle(width: u32, height: u32, color: [u8; 4], radius: f32) -> RgbaImage {
    let mut output = RgbaImage::new(width, height);
    let radius = radius.clamp(0.0, width.min(height) as f32 * 0.5);
    for y in 0..height {
        for x in 0..width {
            let dx = if x as f32 <= radius {
                radius - x as f32
            } else if x as f32 >= width as f32 - radius {
                x as f32 - (width as f32 - radius)
            } else {
                0.0
            };
            let dy = if y as f32 <= radius {
                radius - y as f32
            } else if y as f32 >= height as f32 - radius {
                y as f32 - (height as f32 - radius)
            } else {
                0.0
            };
            if radius == 0.0 || dx * dx + dy * dy <= radius * radius {
                output.put_pixel(x, y, Rgba(color));
            }
        }
    }
    output
}

fn transform_layer(image: DynamicImage, transform: Transform) -> RgbaImage {
    let width = (image.width() as f32 * transform.scale_x).round().max(1.0) as u32;
    let height = (image.height() as f32 * transform.scale_y).round().max(1.0) as u32;
    let scaled = image
        .resize_exact(width, height, FilterType::Triangle)
        .to_rgba8();
    if transform.rotation.abs() < 0.01 {
        return scaled;
    }
    rotate_rgba(&scaled, transform.rotation)
}

fn rotate_rgba(source: &RgbaImage, degrees: f32) -> RgbaImage {
    let radians = degrees.to_radians();
    let (sin, cos) = radians.sin_cos();
    let width = source.width() as f32;
    let height = source.height() as f32;
    let output_width = (width * cos.abs() + height * sin.abs()).ceil().max(1.0) as u32;
    let output_height = (width * sin.abs() + height * cos.abs()).ceil().max(1.0) as u32;
    let source_center = ((width - 1.0) * 0.5, (height - 1.0) * 0.5);
    let output_center = (
        (output_width - 1) as f32 * 0.5,
        (output_height - 1) as f32 * 0.5,
    );
    let mut output = RgbaImage::new(output_width, output_height);
    for y in 0..output_height {
        for x in 0..output_width {
            let dx = x as f32 - output_center.0;
            let dy = y as f32 - output_center.1;
            let source_x = cos * dx + sin * dy + source_center.0;
            let source_y = -sin * dx + cos * dy + source_center.1;
            if source_x >= 0.0 && source_y >= 0.0 && source_x < width && source_y < height {
                let sample_x = source_x.round().clamp(0.0, width - 1.0) as u32;
                let sample_y = source_y.round().clamp(0.0, height - 1.0) as u32;
                output.put_pixel(x, y, *source.get_pixel(sample_x, sample_y));
            }
        }
    }
    output
}

fn composite_layer(
    canvas: &mut RgbaImage,
    coverage: &mut RgbaImage,
    source: &RgbaImage,
    layer: &Layer,
    clip: Option<&RgbaImage>,
) {
    let origin_x = layer.transform.x.round() as i32;
    let origin_y = layer.transform.y.round() as i32;
    for (source_x, source_y, source_pixel) in source.enumerate_pixels() {
        let canvas_x = origin_x + source_x as i32;
        let canvas_y = origin_y + source_y as i32;
        if canvas_x < 0
            || canvas_y < 0
            || canvas_x >= canvas.width() as i32
            || canvas_y >= canvas.height() as i32
        {
            continue;
        }
        let normalized_x = source_x as f32 / source.width().max(1) as f32;
        let normalized_y = source_y as f32 / source.height().max(1) as f32;
        let in_mask = normalized_x >= layer.mask.x
            && normalized_x <= layer.mask.x + layer.mask.width
            && normalized_y >= layer.mask.y
            && normalized_y <= layer.mask.y + layer.mask.height;
        let mask_alpha = if !layer.mask.enabled || in_mask != layer.mask.invert {
            1.0
        } else {
            0.0
        };
        let x = canvas_x as u32;
        let y = canvas_y as u32;
        let clip_alpha = if layer.clip_to_below {
            clip.map_or(0.0, |image| image.get_pixel(x, y)[3] as f32 / 255.0)
        } else {
            1.0
        };
        let alpha = source_pixel[3] as f32 / 255.0 * layer.opacity * mask_alpha * clip_alpha;
        if alpha <= 0.0 {
            continue;
        }
        let destination = *canvas.get_pixel(x, y);
        let blended = blend_rgb(source_pixel.0, destination.0, layer.blend_mode);
        let destination_alpha = destination[3] as f32 / 255.0;
        let output_alpha = alpha + destination_alpha * (1.0 - alpha);
        let mut output = [0; 4];
        for channel in 0..3 {
            let value = if output_alpha > 0.0 {
                (blended[channel] as f32 * alpha
                    + destination[channel] as f32 * destination_alpha * (1.0 - alpha))
                    / output_alpha
            } else {
                0.0
            };
            output[channel] = value.round().clamp(0.0, 255.0) as u8;
        }
        output[3] = (output_alpha * 255.0).round() as u8;
        canvas.put_pixel(x, y, Rgba(output));
        coverage.put_pixel(x, y, Rgba([255, 255, 255, (alpha * 255.0) as u8]));
    }
}

fn blend_rgb(source: [u8; 4], destination: [u8; 4], mode: BlendMode) -> [u8; 3] {
    let blend = |source: u8, destination: u8| -> u8 {
        let s = source as f32 / 255.0;
        let d = destination as f32 / 255.0;
        let value = match mode {
            BlendMode::Normal => s,
            BlendMode::Multiply => s * d,
            BlendMode::Screen => 1.0 - (1.0 - s) * (1.0 - d),
            BlendMode::Overlay => {
                if d <= 0.5 {
                    2.0 * s * d
                } else {
                    1.0 - 2.0 * (1.0 - s) * (1.0 - d)
                }
            }
        };
        (value * 255.0).round().clamp(0.0, 255.0) as u8
    };
    [
        blend(source[0], destination[0]),
        blend(source[1], destination[1]),
        blend(source[2], destination[2]),
    ]
}

pub fn export_document(document: &Document, path: &Path, quality: u8) -> Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !matches!(extension.as_str(), "jpg" | "jpeg" | "png") {
        bail!("export path must end in .png, .jpg, or .jpeg");
    }
    let destination = if path.exists() {
        fs::canonicalize(path)?
    } else {
        let parent = fs::canonicalize(
            path.parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .unwrap_or_else(|| Path::new(".")),
        )?;
        parent.join(path.file_name().context("export path needs a file name")?)
    };
    for layer in &document.layers {
        if let LayerKind::Raster {
            path: source,
            original_path,
        } = &layer.kind
        {
            let overwrites_source = fs::canonicalize(source).ok().as_ref() == Some(&destination);
            let overwrites_original = original_path.as_ref().is_some_and(|original| {
                fs::canonicalize(original).ok().as_ref() == Some(&destination)
            });
            if overwrites_source || overwrites_original {
                bail!(
                    "refusing to overwrite raster source {}; choose a new export path",
                    if overwrites_original {
                        original_path.as_ref().unwrap_or(source)
                    } else {
                        source
                    }
                    .display()
                );
            }
        }
    }
    let image = render_document(document, None)?;
    let file =
        fs::File::create(path).with_context(|| format!("could not create {}", path.display()))?;
    let writer = BufWriter::new(file);
    match extension.as_str() {
        "jpg" | "jpeg" => {
            let rgb = image.to_rgb8();
            image::codecs::jpeg::JpegEncoder::new_with_quality(writer, quality.clamp(1, 100))
                .write_image(
                    &rgb,
                    rgb.width(),
                    rgb.height(),
                    image::ExtendedColorType::Rgb8,
                )?;
        }
        "png" => {
            let rgba = image.to_rgba8();
            image::codecs::png::PngEncoder::new(writer).write_image(
                &rgba,
                rgba.width(),
                rgba.height(),
                image::ExtendedColorType::Rgba8,
            )?;
        }
        _ => unreachable!("extension was validated before rendering"),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn test_directory(label: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("mica-{label}-{stamp}"))
    }

    #[test]
    fn command_history_restores_layers() {
        let mut workspace = Workspace::default();
        workspace
            .execute(Command::AddRectangle {
                name: Some("Card".into()),
                width: 100,
                height: 80,
                color: [255, 0, 0, 255],
                corner_radius: 8.0,
                x: 20.0,
                y: 30.0,
            })
            .unwrap();
        assert_eq!(workspace.document.layers.len(), 1);
        workspace.execute(Command::Undo).unwrap();
        assert!(workspace.document.layers.is_empty());
        workspace.execute(Command::Redo).unwrap();
        assert_eq!(workspace.document.layers.len(), 1);
    }

    #[test]
    fn clipping_uses_layer_below_alpha() {
        let mut document = Document::new("Clip", 20, 20);
        let mut workspace = Workspace::new(document.clone(), None);
        workspace
            .execute(Command::AddRectangle {
                name: None,
                width: 5,
                height: 5,
                color: [255, 255, 255, 255],
                corner_radius: 0.0,
                x: 2.0,
                y: 2.0,
            })
            .unwrap();
        workspace
            .execute(Command::AddRectangle {
                name: None,
                width: 20,
                height: 20,
                color: [255, 0, 0, 255],
                corner_radius: 0.0,
                x: 0.0,
                y: 0.0,
            })
            .unwrap();
        let top = workspace.document.selected.unwrap();
        workspace
            .execute(Command::SetClipping {
                id: top,
                enabled: true,
            })
            .unwrap();
        document = workspace.document;
        document.background = [0, 0, 0, 0];
        let rendered = render_document(&document, None).unwrap().to_rgba8();
        assert_eq!(rendered.get_pixel(3, 3)[0], 255);
        assert_eq!(rendered.get_pixel(10, 10)[3], 0);
    }

    #[test]
    fn text_and_shapes_render() {
        let mut workspace = Workspace::new(Document::new("Poster", 400, 200), None);
        workspace
            .execute(Command::AddText {
                text: "Mica".into(),
                name: None,
                font_size: 48.0,
                color: [255, 255, 255, 255],
                x: 30.0,
                y: 40.0,
            })
            .unwrap();
        let rendered = render_document(&workspace.document, None).unwrap();
        assert_eq!((rendered.width(), rendered.height()), (400, 200));
    }

    #[test]
    fn arbitrary_rotation_never_samples_outside_source() {
        let mut workspace = Workspace::new(Document::new("Rotate", 100, 100), None);
        workspace
            .execute(Command::AddRectangle {
                name: None,
                width: 37,
                height: 23,
                color: [255, 0, 0, 255],
                corner_radius: 0.0,
                x: 10.0,
                y: 10.0,
            })
            .unwrap();
        workspace
            .execute(Command::SetTransform {
                id: 1,
                transform: Transform {
                    x: 10.0,
                    y: 10.0,
                    rotation: 13.0,
                    ..Default::default()
                },
            })
            .unwrap();
        assert!(render_document(&workspace.document, None).is_ok());
    }

    #[test]
    fn export_refuses_to_overwrite_a_raster_source() {
        let directory = test_directory("immutable-source");
        fs::create_dir_all(&directory).unwrap();
        let source = directory.join("original.png");
        RgbaImage::from_pixel(4, 4, Rgba([20, 40, 60, 255]))
            .save(&source)
            .unwrap();
        let original = fs::read(&source).unwrap();
        let mut workspace = Workspace::new(Document::new("Safety", 4, 4), None);
        workspace
            .execute(Command::AddRaster {
                path: source.clone(),
                name: None,
                x: 0.0,
                y: 0.0,
            })
            .unwrap();
        assert!(export_document(&workspace.document, &source, 92).is_err());
        assert_eq!(fs::read(&source).unwrap(), original);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn non_finite_commands_are_rejected_before_serialization() {
        let mut workspace = Workspace::new(Document::new("Finite", 20, 20), None);
        workspace
            .execute(Command::AddRectangle {
                name: None,
                width: 10,
                height: 10,
                color: [255, 255, 255, 255],
                corner_radius: 0.0,
                x: 0.0,
                y: 0.0,
            })
            .unwrap();
        assert!(
            workspace
                .execute(Command::SetOpacity {
                    id: 1,
                    opacity: f32::NAN,
                })
                .is_err()
        );
        assert!(workspace.document.layer(1).unwrap().opacity.is_finite());
        assert!(
            !serde_json::to_string(&workspace.document)
                .unwrap()
                .contains("\"opacity\":null")
        );
    }

    #[test]
    fn preview_renders_at_target_size_without_full_canvas_allocation() {
        let document = Document::new("Large", MAX_CANVAS_DIMENSION, MAX_CANVAS_DIMENSION);
        let preview = render_document(&document, Some(512)).unwrap();
        assert_eq!((preview.width(), preview.height()), (512, 512));
    }

    #[test]
    fn selection_is_command_driven_but_not_an_undo_step() {
        let mut workspace = Workspace::new(Document::new("Selection", 20, 20), None);
        workspace
            .execute(Command::AddRectangle {
                name: None,
                width: 10,
                height: 10,
                color: [255, 255, 255, 255],
                corner_radius: 0.0,
                x: 0.0,
                y: 0.0,
            })
            .unwrap();
        workspace
            .execute(Command::SelectLayer { id: None })
            .unwrap();
        workspace.execute(Command::Undo).unwrap();
        assert!(workspace.document.layers.is_empty());
    }

    #[test]
    fn saving_copies_external_sources_into_portable_assets() {
        let root = test_directory("portable");
        let source_directory = root.join("external");
        let project_directory = root.join("project");
        fs::create_dir_all(&source_directory).unwrap();
        fs::create_dir_all(&project_directory).unwrap();
        let source = source_directory.join("source.png");
        RgbaImage::from_pixel(2, 2, Rgba([1, 2, 3, 255]))
            .save(&source)
            .unwrap();
        let project = project_directory.join("portable.mica");
        let mut workspace = Workspace::new(Document::new("Portable", 2, 2), Some(project.clone()));
        workspace
            .execute(Command::AddRaster {
                path: source.clone(),
                name: None,
                x: 0.0,
                y: 0.0,
            })
            .unwrap();
        workspace.save(None).unwrap();
        let LayerKind::Raster {
            path,
            original_path,
        } = &workspace.document.layers[0].kind
        else {
            panic!("expected raster layer");
        };
        assert!(path.starts_with(fs::canonicalize(&project_directory).unwrap()));
        assert!(path.exists());
        assert_eq!(
            original_path.as_ref(),
            Some(&fs::canonicalize(&source).unwrap())
        );
        assert!(export_document(&workspace.document, &source, 90).is_err());
        let serialized = fs::read_to_string(project).unwrap();
        assert!(serialized.contains("portable-assets"));
        fs::remove_dir_all(root).unwrap();
    }
}
