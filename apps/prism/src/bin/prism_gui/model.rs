use super::*;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Tool {
    #[default]
    Move,
    Rotate,
    Crop,
    Text,
    Shape,
    Mask,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ToolActivation {
    ChoiceDialog,
    CanvasGesture,
}

impl Tool {
    pub(super) const ALL: [(Self, &'static str, &'static str); 5] = [
        (Self::Move, "V", "Select / move"),
        (Self::Crop, "C", "Crop canvas"),
        (Self::Text, "T", "Text"),
        (Self::Shape, "S", "Shape"),
        (Self::Mask, "M", "Layer mask"),
    ];

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Move => "Move",
            Self::Rotate => "Rotate",
            Self::Crop => "Crop canvas",
            Self::Text => "Add text",
            Self::Shape => "Shape",
            Self::Mask => "Draw mask",
        }
    }

    pub(super) fn shortcut(self) -> &'static str {
        if self == Self::Rotate {
            return "R";
        }
        Self::ALL
            .iter()
            .find_map(|(tool, key, _)| (*tool == self).then_some(*key))
            .unwrap_or_default()
    }

    pub(super) fn description(self) -> &'static str {
        match self {
            Self::Move => "Select on the canvas, drag to move, or pull a corner to resize.",
            Self::Rotate => "Rotation armed · drag around the focused object · Shift snaps to 15°.",
            Self::Crop => "Draw the new canvas boundary.",
            Self::Text => "Click for point text, or drag a width for a wrapped paragraph.",
            Self::Shape => "Choose a rectangle, ellipse, or another shape to draw.",
            Self::Mask => "Draw the visible region of the focused element.",
        }
    }

    pub(super) fn activation(self) -> ToolActivation {
        match self {
            Self::Shape => ToolActivation::ChoiceDialog,
            _ => ToolActivation::CanvasGesture,
        }
    }

    pub(super) fn matches(self, query: &str) -> bool {
        let query = query.trim().to_ascii_lowercase();
        query.is_empty()
            || self.label().to_ascii_lowercase().contains(&query)
            || self.description().to_ascii_lowercase().contains(&query)
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct CanvasGeometry {
    pub viewport: Rect,
    pub canvas: Rect,
    pub pixels_per_point: f32,
}

impl CanvasGeometry {
    pub(super) fn screen_to_canvas(self, position: Pos2) -> Pos2 {
        Pos2::new(
            (position.x - self.canvas.left()) / self.pixels_per_point,
            (position.y - self.canvas.top()) / self.pixels_per_point,
        )
    }

    pub(super) fn canvas_to_screen(self, position: Pos2) -> Pos2 {
        self.canvas.min + position.to_vec2() * self.pixels_per_point
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct DragState {
    pub start_canvas: Pos2,
    pub current_canvas: Pos2,
    pub layer_id: Option<u64>,
    pub transform: Transform,
    pub action: DragAction,
    pub bounds: Option<Rect>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DragAction {
    Move,
    Rotate,
    Resize(ResizeHandle),
    Guide(u64),
    Draw,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ResizeHandle {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CanvasInvalidation {
    None,
    Layer(u64),
    Structure,
    All,
}

pub(super) fn canvas_invalidation(command: &Command) -> CanvasInvalidation {
    match command {
        Command::UpdateText { id, .. }
        | Command::SetTextTypography { id, .. }
        | Command::UpdateRectangle { id, .. }
        | Command::UpdateEllipse { id, .. }
        | Command::SetShapeStroke { id, .. }
        | Command::RasterizeShape { id, .. }
        | Command::AdjustLayer { id, .. }
        | Command::ResetLayerAdjustments { id } => CanvasInvalidation::Layer(*id),
        Command::AddRaster { .. }
        | Command::AddText { .. }
        | Command::AddRectangle { .. }
        | Command::AddEllipse { .. }
        | Command::InsertLayer { .. }
        | Command::DuplicateLayer { .. }
        | Command::Undo
        | Command::Redo => CanvasInvalidation::All,
        Command::ImportFont { .. } => CanvasInvalidation::None,
        Command::RemoveLayer { .. } => CanvasInvalidation::Structure,
        _ => CanvasInvalidation::None,
    }
}
