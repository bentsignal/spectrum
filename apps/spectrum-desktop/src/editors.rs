//! State for each editor: the image editor's color and crop controls and
//! open image, and the canvas editor's layer and font controls. The open
//! canvas itself is `CanvasState`.
use gpui::*;
use gpui_component::{input::InputState, slider::SliderState};
use std::{cell::RefCell, rc::Rc};

pub struct ImageEditor {
    /// Real color correction sliders, in `color_fields::FIELDS` order.
    pub color: Vec<Entity<SliderState>>,
    /// The open image's adjustments, sent whole to the engine on each change.
    pub adjust: spectrum_image::Adjustments,
    pub color_section: usize,
    pub mix_band: usize,
    pub grade_range: usize,
    pub curve_channel: usize,
    pub curve_drag: Option<usize>,
    pub curve_bounds: Rc<RefCell<Bounds<Pixels>>>,
    pub straighten: Entity<SliderState>,
    /// The open image, rendered in memory as it is edited.
    pub preview: Option<crate::preview::Preview>,
    /// The open image's saves and history.
    pub edits: crate::preview::ImageEdits,
    pub crop_drag: Option<crate::crop::CropDrag>,
    pub crop_aspect: usize,
    pub image_bounds: Rc<RefCell<Bounds<Pixels>>>,
    /// A color edit is running; `color_dirty` asks for another when it ends.
    pub color_busy: bool,
    pub color_dirty: bool,
    /// The open image's record, for Info mode.
    pub photo_info: Option<spectrum_image::Image>,
    /// Shows the unedited image beside the edited one.
    pub compare: bool,
}

pub struct CanvasControls {
    /// Installed font families, loaded when the font browser first opens.
    pub fonts: Option<std::sync::Arc<Vec<crate::font_browser::Family>>>,
    pub font_open: bool,
    /// Font files that Spectrum will not embed, found while previewing.
    pub font_blocked: std::collections::HashSet<std::path::PathBuf>,
    pub font_query: Entity<InputState>,
    pub font_highlight: usize,
    pub font_hover: Option<usize>,
    pub font_scroll: UniformListScrollHandle,
    pub font_lists: crate::font_lists::FontLists,
    /// The installed fonts are being found.
    pub fonts_requested: bool,
    /// Style controls for the selected layer.
    pub opacity: Entity<SliderState>,
    pub text_input: Entity<InputState>,
    pub text_size: Entity<SliderState>,
    pub corner: Entity<SliderState>,
    pub rotation: Entity<SliderState>,
    pub line_height: Entity<SliderState>,
    pub tracking: Entity<SliderState>,
    pub styles: crate::layer_styles::StyleControls,
    pub tool_options: crate::tool_options::ToolOptions,
    pub fill_gradient: crate::gradient_editor::GradientEditor,
    pub overlay_gradient: crate::gradient_editor::GradientEditor,
    pub style_section: usize,
    /// Guides show on canvases; hiding them also stops dragging them.
    pub guides_visible: bool,
}
