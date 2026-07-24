use super::*;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Tool {
    #[default]
    Move,
    Rotate,
    Crop,
    Text,
    Shape,
    Pen,
    Brush,
    Eraser,
    Mask,
    Marquee,
    Lasso,
    MagicWand,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ToolActivation {
    ChoiceDialog,
    CanvasGesture,
}

impl Tool {
    pub(super) const ALL: [(Self, &'static str, &'static str); 11] = [
        (Self::Move, "V", "Select / move"),
        (Self::Crop, "C", "Crop canvas"),
        (Self::Text, "T", "Text"),
        (Self::Shape, "S", "Shape"),
        (Self::Pen, "P", "Pen path"),
        (Self::Brush, "B", "Brush"),
        (Self::Eraser, "E", "Eraser"),
        (Self::Mask, "K", "Layer mask"),
        (Self::Marquee, "M", "Selection"),
        (Self::Lasso, "L", "Freehand lasso"),
        (Self::MagicWand, "W", "Magic wand"),
    ];

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Move => "Move",
            Self::Rotate => "Rotate",
            Self::Crop => "Crop canvas",
            Self::Text => "Add text",
            Self::Shape => "Shape",
            Self::Pen => "Pen",
            Self::Brush => "Brush",
            Self::Eraser => "Eraser",
            Self::Mask => "Draw mask",
            Self::Marquee => "Selection",
            Self::Lasso => "Freehand lasso",
            Self::MagicWand => "Magic wand",
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
            Self::Pen => {
                "Click anchors, drag cubic handles, click the first anchor to close, or press Enter for an open path."
            }
            Self::Brush => "Drag to paint one nondestructive stroke on a Paint layer.",
            Self::Eraser => "Drag to erase nondestructively within the focused Paint layer.",
            Self::Mask => "Draw the visible region of the focused element.",
            Self::Marquee => "Drag a persistent document-pixel selection, then clear or fill it.",
            Self::Lasso => {
                "Drag a freehand selection; Shift adds, Option subtracts, and both intersect."
            }
            Self::MagicWand => "Click a color to select a connected or canvas-wide range.",
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
    pub paragraph_bounds: Option<Rect>,
    pub paragraph_width: Option<f32>,
    pub paragraph_source_override: Option<LayerSourceGeometry>,
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
    ParagraphLeft,
    ParagraphRight,
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
        | Command::ReplacePath { id, .. }
        | Command::SetVectorMask { id, .. }
        | Command::SetShapeStroke { id, .. }
        | Command::SetShapeFill { id, .. }
        | Command::SetLayerStyle { id, .. }
        | Command::RasterizeShape { id, .. }
        | Command::AdjustLayer { id, .. }
        | Command::ResetLayerAdjustments { id }
        | Command::AddBrushStroke { id, .. }
        | Command::DeleteSelectedPixels { id } => CanvasInvalidation::Layer(*id),
        Command::AddRaster { .. }
        | Command::AddText { .. }
        | Command::AddRectangle { .. }
        | Command::AddEllipse { .. }
        | Command::AddPath { .. }
        | Command::AddPaintLayer { .. }
        | Command::AddPaintLayerWithStroke { .. }
        | Command::FillSelection { .. }
        | Command::InsertLayer { .. }
        | Command::DuplicateLayer { .. }
        | Command::Undo
        | Command::Redo => CanvasInvalidation::All,
        Command::ImportFont { .. }
        | Command::SetSelection { .. }
        | Command::CropCanvas { .. }
        | Command::CropToSelection => CanvasInvalidation::None,
        Command::RemoveLayer { .. } => CanvasInvalidation::Structure,
        _ => CanvasInvalidation::None,
    }
}

pub(super) fn text_geometry_invalidation(command: &Command) -> Option<u64> {
    match command {
        Command::UpdateText { id, .. } | Command::SetTextTypography { id, .. } => Some(*id),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stroke() -> prism_core::BrushStroke {
        prism_core::BrushStroke::new(
            prism_core::BrushStyle::default(),
            vec![prism_core::BrushSample {
                x: 1.5,
                y: 1.5,
                pressure: 1.0,
            }],
        )
        .unwrap()
    }

    #[test]
    fn paint_commands_invalidate_structures_and_cached_layer_pixels() {
        assert_eq!(
            canvas_invalidation(&Command::AddPaintLayer {
                name: None,
                width: 10,
                height: 10,
            }),
            CanvasInvalidation::All
        );
        assert_eq!(
            canvas_invalidation(&Command::AddPaintLayerWithStroke {
                name: None,
                width: 10,
                height: 10,
                stroke: stroke(),
                selection: prism_core::PaintSelection::None,
            }),
            CanvasInvalidation::All
        );
        assert_eq!(
            canvas_invalidation(&Command::AddBrushStroke {
                id: 42,
                stroke: stroke(),
                selection: prism_core::PaintSelection::None,
            }),
            CanvasInvalidation::Layer(42)
        );
    }

    #[test]
    fn text_geometry_survives_transforms_but_not_text_or_font_identity_edits() {
        assert_eq!(
            text_geometry_invalidation(&Command::SetTransform {
                id: 42,
                transform: Transform::default(),
            }),
            None
        );
        assert_eq!(
            text_geometry_invalidation(&Command::UpdateText {
                id: 42,
                text: "Changed".into(),
                font_size: 72.0,
                color: [255; 4],
            }),
            Some(42)
        );
        assert_eq!(
            text_geometry_invalidation(&Command::SetTextTypography {
                id: 42,
                typography: prism_core::TextTypography {
                    font_id: Some(7),
                    ..Default::default()
                },
            }),
            Some(42)
        );
    }

    #[test]
    fn command_invalidation_keeps_transform_and_appearance_on_the_gpu() {
        assert_eq!(
            canvas_invalidation(&Command::SetTransform {
                id: 7,
                transform: Transform::default(),
            }),
            CanvasInvalidation::None
        );
        assert_eq!(
            canvas_invalidation(&Command::SetOpacity {
                id: 7,
                opacity: 0.5,
            }),
            CanvasInvalidation::None
        );
        assert_eq!(
            canvas_invalidation(&Command::AdjustLayer {
                id: 7,
                patch: spectrum_imaging::AdjustmentPatch {
                    exposure: Some(1.0),
                    ..Default::default()
                },
            }),
            CanvasInvalidation::Layer(7)
        );
        assert_eq!(
            canvas_invalidation(&Command::SetTextTypography {
                id: 7,
                typography: prism_core::TextTypography::default(),
            }),
            CanvasInvalidation::Layer(7)
        );
        assert_eq!(
            canvas_invalidation(&Command::ImportFont {
                path: std::path::PathBuf::from("face.otf"),
                source_name: None,
            }),
            CanvasInvalidation::None
        );
        assert_eq!(canvas_invalidation(&Command::Undo), CanvasInvalidation::All);
    }
}
