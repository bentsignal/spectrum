//! Prism's command-driven layered document engine.

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use spectrum_imaging::{AdjustmentPatch, Adjustments};

pub const PRISM_VERSION: u32 = 1;
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
    Darken,
    Lighten,
    ColorDodge,
    ColorBurn,
    HardLight,
    SoftLight,
    Difference,
    Exclusion,
}

impl BlendMode {
    pub const ALL: [Self; 12] = [
        Self::Normal,
        Self::Multiply,
        Self::Screen,
        Self::Overlay,
        Self::Darken,
        Self::Lighten,
        Self::ColorDodge,
        Self::ColorBurn,
        Self::HardLight,
        Self::SoftLight,
        Self::Difference,
        Self::Exclusion,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Normal => "Normal",
            Self::Multiply => "Multiply",
            Self::Screen => "Screen",
            Self::Overlay => "Overlay",
            Self::Darken => "Darken",
            Self::Lighten => "Lighten",
            Self::ColorDodge => "Color Dodge",
            Self::ColorBurn => "Color Burn",
            Self::HardLight => "Hard Light",
            Self::SoftLight => "Soft Light",
            Self::Difference => "Difference",
            Self::Exclusion => "Exclusion",
        }
    }

    pub const fn description(self) -> &'static str {
        match self {
            Self::Normal => "Uses the layer color without blending it with layers below.",
            Self::Multiply => "Darkens by multiplying colors; white has no effect.",
            Self::Screen => "Lightens like projected light; black has no effect.",
            Self::Overlay => "Boosts contrast using Multiply on shadows and Screen on highlights.",
            Self::Darken => "Keeps the darker value from this layer or the layers below.",
            Self::Lighten => "Keeps the lighter value from this layer or the layers below.",
            Self::ColorDodge => "Brightens the layers below and can create intense highlights.",
            Self::ColorBurn => "Darkens the layers below and increases shadow contrast.",
            Self::HardLight => "Uses a strong light based on this layer's brightness.",
            Self::SoftLight => "Adds a gentle contrast and lighting effect.",
            Self::Difference => "Shows the absolute difference between the two colors.",
            Self::Exclusion => "Creates a softer, lower-contrast Difference effect.",
        }
    }
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
            version: PRISM_VERSION,
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
        if self.version > PRISM_VERSION {
            bail!(
                "project version {} is newer than this app supports ({PRISM_VERSION})",
                self.version
            );
        }
        self.version = PRISM_VERSION;
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
    interaction_before: Option<Document>,
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
            interaction_before: None,
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
            .context("choose a .prism project path first")?;
        save_document(&self.document, &path)?;
        self.document = load_document(&path)?;
        self.project_path = Some(path.clone());
        self.dirty = false;
        Ok(path)
    }

    pub fn execute(&mut self, command: Command) -> Result<CommandOutput> {
        if self.interaction_before.is_some() {
            bail!("finish the active interaction before executing another command");
        }
        match command {
            Command::Undo => self.undo(),
            Command::Redo => self.redo(),
            command @ Command::SelectLayer { .. } => apply_command(&mut self.document, command),
            command => {
                let before = self.document.clone();
                let output = apply_command(&mut self.document, command)?;
                if self.document != before {
                    self.record_edit(before);
                }
                Ok(output)
            }
        }
    }

    /// Starts a gesture whose many preview commands should become one undo step.
    pub fn begin_interaction(&mut self) {
        if self.interaction_before.is_none() {
            self.interaction_before = Some(self.document.clone());
        }
    }

    /// Applies a command without cloning the document or adding an undo entry.
    /// Call [`Workspace::commit_interaction`] when the gesture ends.
    pub fn preview(&mut self, command: Command) -> Result<CommandOutput> {
        if self.interaction_before.is_none() {
            bail!("begin an interaction before applying preview commands");
        }
        if matches!(
            command,
            Command::Undo | Command::Redo | Command::SelectLayer { .. }
        ) {
            bail!("history and selection commands cannot preview an interaction");
        }
        apply_command(&mut self.document, command)
    }

    /// Commits the current gesture as a single history entry.
    pub fn commit_interaction(&mut self) -> bool {
        let Some(before) = self.interaction_before.take() else {
            return false;
        };
        if self.document == before {
            return false;
        }
        self.record_edit(before);
        true
    }

    /// Restores the document from before the current gesture.
    pub fn cancel_interaction(&mut self) -> bool {
        let Some(before) = self.interaction_before.take() else {
            return false;
        };
        self.document = before;
        true
    }

    pub fn interaction_active(&self) -> bool {
        self.interaction_before.is_some()
    }

    fn record_edit(&mut self, before: Document) {
        self.undo.push(before);
        if self.undo.len() > MAX_HISTORY {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.dirty = true;
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

mod render;

pub use render::{
    export_document, load_document, measure_text, render_document, render_layer_base,
    render_layer_preview, render_solid_color, save_document,
};

#[cfg(test)]
#[path = "core_tests.rs"]
mod tests;
