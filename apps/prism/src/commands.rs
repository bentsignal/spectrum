use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use spectrum_imaging::AdjustmentPatch;

use crate::{
    Alignment, AlignmentReference, BlendMode, BrushStroke, GuideOrientation, LassoPath, LayerMask,
    LayerStyle, LayerTransfer, PathGeometry, Selection, SelectionCombineMode, ShapeFill,
    ShapeStroke, TextTypography, Transform, VectorMask,
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum PaintSelection {
    Current,
    None,
    Snapshot { selection: Box<Selection> },
}

#[derive(Clone, Debug, Serialize)]
pub struct CommandOutput {
    pub action: String,
    pub message: String,
    pub layer_ids: Vec<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub guide_ids: Vec<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum Command {
    RenameDocument {
        name: String,
    },
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
    CropToSelection,
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
    ImportFont {
        path: PathBuf,
        #[serde(default)]
        source_name: Option<String>,
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
    AddEllipse {
        name: Option<String>,
        width: u32,
        height: u32,
        color: [u8; 4],
        x: f32,
        y: f32,
    },
    AddPath {
        name: Option<String>,
        geometry: PathGeometry,
        color: [u8; 4],
        x: f32,
        y: f32,
    },
    AddPaintLayer {
        name: Option<String>,
        width: u32,
        height: u32,
    },
    AddPaintLayerWithStroke {
        name: Option<String>,
        width: u32,
        height: u32,
        stroke: BrushStroke,
        selection: PaintSelection,
    },
    AddBrushStroke {
        id: u64,
        stroke: BrushStroke,
        selection: PaintSelection,
    },
    UpdateText {
        id: u64,
        text: String,
        font_size: f32,
        color: [u8; 4],
    },
    SetTextTypography {
        id: u64,
        typography: TextTypography,
    },
    UpdateRectangle {
        id: u64,
        width: u32,
        height: u32,
        color: [u8; 4],
        corner_radius: f32,
    },
    UpdateEllipse {
        id: u64,
        width: u32,
        height: u32,
        color: [u8; 4],
    },
    ReplacePath {
        id: u64,
        geometry: PathGeometry,
    },
    RemoveLayer {
        id: u64,
    },
    DuplicateLayer {
        id: u64,
    },
    InsertLayer {
        transfer: Box<LayerTransfer>,
        #[serde(default)]
        index: Option<usize>,
    },
    RenameLayer {
        id: u64,
        name: String,
    },
    SelectLayer {
        id: Option<u64>,
    },
    SetSelection {
        selection: Option<Selection>,
    },
    MagicWandSelection {
        x: u32,
        y: u32,
        tolerance: u8,
        contiguous: bool,
        antialias: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        resolved_selection: Option<Box<Selection>>,
    },
    /// Durable marker whose exact result is carried by a required same-revision snapshot.
    MagicWandSnapshot {
        x: u32,
        y: u32,
        tolerance: u8,
        contiguous: bool,
        antialias: bool,
    },
    LassoSelection {
        points: LassoPath,
        mode: SelectionCombineMode,
        antialias: bool,
    },
    FillSelection {
        color: [u8; 4],
        name: Option<String>,
    },
    DeleteSelectedPixels {
        id: u64,
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
    SetDissolveSeed {
        id: u64,
        seed: u32,
    },
    SetTransform {
        id: u64,
        transform: Transform,
    },
    SetRotation {
        id: u64,
        degrees: f32,
    },
    AlignLayer {
        id: u64,
        alignment: Alignment,
        reference: AlignmentReference,
    },
    SetSnapping {
        enabled: bool,
    },
    AddGuide {
        orientation: GuideOrientation,
        position: f32,
    },
    MoveGuide {
        id: u64,
        position: f32,
    },
    RemoveGuide {
        id: u64,
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
    SetVectorMask {
        id: u64,
        mask: Option<VectorMask>,
    },
    SetShapeStroke {
        id: u64,
        stroke: ShapeStroke,
    },
    SetLayerStyle {
        id: u64,
        style: LayerStyle,
    },
    SetShapeFill {
        id: u64,
        fill: Option<ShapeFill>,
    },
    RasterizeShape {
        id: u64,
        path: PathBuf,
        scale: f32,
    },
    SetClipping {
        id: u64,
        enabled: bool,
    },
    Undo,
    Redo,
}

pub(crate) fn output(action: &str, message: &str, layer_ids: Vec<u64>) -> CommandOutput {
    CommandOutput {
        action: action.into(),
        message: message.into(),
        layer_ids,
        guide_ids: Vec::new(),
        warnings: Vec::new(),
    }
}

pub(crate) fn guide_output(action: &str, message: &str, guide_ids: Vec<u64>) -> CommandOutput {
    CommandOutput {
        action: action.into(),
        message: message.into(),
        layer_ids: Vec::new(),
        guide_ids,
        warnings: Vec::new(),
    }
}
