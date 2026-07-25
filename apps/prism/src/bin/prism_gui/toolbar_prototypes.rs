use super::*;
use crate::chrome::{
    PaletteState, ShapeKind,
    chrome_shortcut::{WORKBENCH_ACTION_SIZE, shortcut_action_button},
};

const PROTOTYPE_ENV: &str = "PRISM_TOOLBAR_OVERFLOW_PROTOTYPES";
const PROTOTYPE_WIDTH_ENV: &str = "PRISM_TOOLBAR_PROTOTYPE_WIDTH";
const NARROW_WORKBENCH_HEIGHT: f32 = 88.0;
const VERY_NARROW_WORKBENCH_HEIGHT: f32 = 122.0;
const GRADIENT_POPOVER_WIDTH: f32 = 340.0;
const GRADIENT_POPOVER_HEIGHT: f32 = 480.0;
const GRADIENT_POPOVER_MARGIN: f32 = 8.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum ToolbarOverflowPrototype {
    #[default]
    TrailingMore,
    ScrollRail,
    AdaptiveWrap,
}

impl ToolbarOverflowPrototype {
    const ALL: [Self; 3] = [Self::TrailingMore, Self::ScrollRail, Self::AdaptiveWrap];

    fn key(self) -> &'static str {
        match self {
            Self::TrailingMore => "A",
            Self::ScrollRail => "B",
            Self::AdaptiveWrap => "C",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::TrailingMore => "A · Trailing More",
            Self::ScrollRail => "B · Scroll rail",
            Self::AdaptiveWrap => "C · Adaptive wrap",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::TrailingMore => {
                "Keep the primary tool cluster fixed; move contextual controls into a vertical More menu only when needed."
            }
            Self::ScrollRail => {
                "Keep the single-row toolbar and expose clipped content with persistent edge buttons plus a visible scrollbar."
            }
            Self::AdaptiveWrap => {
                "Keep every control visible and preserve source focus order by wrapping contextual groups onto additional rows."
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ScrollEdge {
    Start,
    End,
}

#[derive(Clone, Copy, Debug)]
struct GradientEditorPanelState {
    layer_id: u64,
    anchor: egui::Pos2,
    trigger_rect: egui::Rect,
}

#[derive(Debug)]
pub(super) struct ToolbarPrototypeState {
    enabled: bool,
    selected: ToolbarOverflowPrototype,
    forced_width: Option<f32>,
    scroll_edge: Option<ScrollEdge>,
    gradient_editor_panel: Option<GradientEditorPanelState>,
}

impl ToolbarPrototypeState {
    pub(super) fn from_environment() -> Self {
        let value = std::env::var(PROTOTYPE_ENV).ok();
        let enabled = prototype_enabled(value.as_deref());
        let selected = match value.as_deref().map(str::trim) {
            Some("B" | "b" | "scroll") => ToolbarOverflowPrototype::ScrollRail,
            Some("C" | "c" | "wrap") => ToolbarOverflowPrototype::AdaptiveWrap,
            _ => ToolbarOverflowPrototype::TrailingMore,
        };
        let forced_width = std::env::var(PROTOTYPE_WIDTH_ENV)
            .ok()
            .and_then(|value| value.parse::<f32>().ok())
            .filter(|width| matches!(*width as u32, 980 | 1200 | 1920));
        Self {
            enabled,
            selected,
            forced_width,
            scroll_edge: None,
            gradient_editor_panel: None,
        }
    }

    pub(super) fn enabled(&self) -> bool {
        self.enabled
    }
}

fn prototype_enabled(value: Option<&str>) -> bool {
    !matches!(
        value.map(str::trim),
        Some("0" | "false" | "off" | "disabled")
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ContextKind {
    None,
    Selection,
    Lasso,
    MagicWand,
    Shape,
    ShapeWithSelection,
    Text,
    TextWithSelection,
    Brush,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ToolbarControl {
    ShapeKind,
    GradientEditor,
    Paragraph,
    Selection,
    Brush,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SelectedLayerKind {
    Shape,
    Text,
    Other,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ControlAccess {
    Inline,
    MoreMenu,
    ScrollRail,
    Wrapped,
    GradientPopover,
    MoreMenuThenGradientPopover,
}

fn control_manifest(
    tool: Tool,
    selected_layer: Option<SelectedLayerKind>,
    has_selection: bool,
) -> Vec<ToolbarControl> {
    let mut controls = Vec::with_capacity(3);
    if tool == Tool::Shape {
        controls.push(ToolbarControl::ShapeKind);
        if selected_layer == Some(SelectedLayerKind::Shape) {
            controls.push(ToolbarControl::GradientEditor);
        }
    }
    if tool == Tool::Text && selected_layer == Some(SelectedLayerKind::Text) {
        controls.push(ToolbarControl::Paragraph);
    }
    if matches!(tool, Tool::Marquee | Tool::Lasso | Tool::MagicWand) || has_selection {
        controls.push(ToolbarControl::Selection);
    }
    if matches!(tool, Tool::Brush | Tool::Eraser) {
        controls.push(ToolbarControl::Brush);
    }
    controls
}

fn selected_layer_kind(layer: &Layer) -> SelectedLayerKind {
    match &layer.kind {
        LayerKind::Rectangle { .. } | LayerKind::Ellipse { .. } => SelectedLayerKind::Shape,
        LayerKind::Path { geometry, .. } if geometry.closed() => SelectedLayerKind::Shape,
        LayerKind::Text { .. } => SelectedLayerKind::Text,
        _ => SelectedLayerKind::Other,
    }
}

#[cfg(test)]
fn access_for(presentation: ResponsivePresentation, control: ToolbarControl) -> ControlAccess {
    match (presentation, control) {
        (ResponsivePresentation::MoreMenu, ToolbarControl::GradientEditor) => {
            ControlAccess::MoreMenuThenGradientPopover
        }
        (_, ToolbarControl::GradientEditor) => ControlAccess::GradientPopover,
        (ResponsivePresentation::Inline, _) => ControlAccess::Inline,
        (ResponsivePresentation::MoreMenu, _) => ControlAccess::MoreMenu,
        (ResponsivePresentation::ScrollRail, _) => ControlAccess::ScrollRail,
        (ResponsivePresentation::Wrapped { .. }, _) => ControlAccess::Wrapped,
    }
}

impl ContextKind {
    fn required_width(self) -> f32 {
        match self {
            Self::None => 760.0,
            Self::Selection => 1_230.0,
            Self::Lasso => 1_460.0,
            Self::MagicWand => 1_530.0,
            Self::Shape => 1_020.0,
            Self::ShapeWithSelection => 1_520.0,
            Self::Text => 1_020.0,
            Self::TextWithSelection => 1_680.0,
            Self::Brush => 1_020.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ResponsivePresentation {
    Inline,
    MoreMenu,
    ScrollRail,
    Wrapped { rows: usize },
}

fn presentation_for(
    prototype: ToolbarOverflowPrototype,
    available_width: f32,
    context: ContextKind,
) -> ResponsivePresentation {
    if available_width >= context.required_width() {
        return ResponsivePresentation::Inline;
    }
    match prototype {
        ToolbarOverflowPrototype::TrailingMore => ResponsivePresentation::MoreMenu,
        ToolbarOverflowPrototype::ScrollRail => ResponsivePresentation::ScrollRail,
        ToolbarOverflowPrototype::AdaptiveWrap => ResponsivePresentation::Wrapped {
            rows: if available_width < 1_080.0 { 3 } else { 2 },
        },
    }
}

fn prototype_widths(available_width: f32, forced_width: Option<f32>) -> (f32, f32) {
    let presentation_width = forced_width.unwrap_or(available_width);
    (presentation_width.min(available_width), presentation_width)
}

impl PrismApp {
    pub(super) fn toolbar_prototype_height(&self, available_width: f32) -> f32 {
        if !self.toolbar_prototype.enabled() {
            return WORKBENCH_HEIGHT;
        }
        match presentation_for(
            self.toolbar_prototype.selected,
            self.toolbar_prototype
                .forced_width
                .unwrap_or(available_width),
            self.toolbar_context_kind(),
        ) {
            ResponsivePresentation::Wrapped { rows: 3 } => VERY_NARROW_WORKBENCH_HEIGHT,
            ResponsivePresentation::Wrapped { .. } => NARROW_WORKBENCH_HEIGHT,
            ResponsivePresentation::ScrollRail => 48.0,
            _ => WORKBENCH_HEIGHT,
        }
    }

    pub(super) fn toolbar_prototype_banner(&mut self, root: &mut egui::Ui) {
        if !self.toolbar_prototype.enabled() {
            return;
        }
        egui::Panel::top("toolbar-overflow-prototype-banner")
            .exact_size(42.0)
            .frame(
                egui::Frame::new()
                    .fill(SELECTED_SURFACE)
                    .inner_margin(egui::Margin::symmetric(8, 6))
                    .stroke(Stroke::new(1.0, ACCENT_WARM)),
            )
            .show(root, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new("PROTOTYPES · DO NOT MERGE")
                            .size(10.0)
                            .strong()
                            .color(ACCENT_WARM),
                    );
                    for prototype in ToolbarOverflowPrototype::ALL {
                        if ui
                            .selectable_label(
                                self.toolbar_prototype.selected == prototype,
                                prototype.label(),
                            )
                            .on_hover_text(prototype.description())
                            .clicked()
                        {
                            self.toolbar_prototype.selected = prototype;
                        }
                    }
                    ui.separator();
                    ui.label(RichText::new("WIDTH").size(9.0).strong().color(SUBTLE));
                    for (label, width) in [
                        ("Live", None),
                        ("980", Some(980.0)),
                        ("1200", Some(1_200.0)),
                        ("1920", Some(1_920.0)),
                    ] {
                        if ui
                            .selectable_label(self.toolbar_prototype.forced_width == width, label)
                            .on_hover_text(
                                "Model this responsive width while keeping rendering inside the native review window",
                            )
                            .clicked()
                        {
                            self.toolbar_prototype.forced_width = width;
                        }
                    }
                    ui.separator();
                    for (label, tool) in [
                        ("Magic Wand", Tool::MagicWand),
                        ("Shape", Tool::Shape),
                        ("Text", Tool::Text),
                        ("Selection", Tool::Marquee),
                    ] {
                        if ui
                            .selectable_label(self.tool == tool, label)
                            .on_hover_text("Switch the actual Prism canvas tool")
                            .clicked()
                        {
                            if tool == Tool::Shape {
                                self.tool = Tool::Shape;
                                self.drag = None;
                                self.status = Tool::Shape.description().into();
                                self.status_error = false;
                            } else {
                                self.choose_tool(tool);
                            }
                        }
                    }
                    ui.separator();
                    ui.label(
                        RichText::new(format!(
                            "Option {} · live {} context",
                            self.toolbar_prototype.selected.key(),
                            self.tool.label()
                        ))
                        .size(10.0)
                        .color(MUTED),
                    );
                });
            });
    }

    pub(super) fn prototype_workbench_contents(&mut self, ui: &mut egui::Ui) {
        let context = ui.ctx().clone();
        let (allocation_width, presentation_width) =
            prototype_widths(ui.available_width(), self.toolbar_prototype.forced_width);
        if self.toolbar_prototype.forced_width.is_some() {
            let height = ui.available_height();
            ui.allocate_ui_with_layout(
                Vec2::new(allocation_width, height),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| self.prototype_workbench_contents_at_width(ui, presentation_width),
            );
        } else {
            self.prototype_workbench_contents_at_width(ui, presentation_width);
        }
        self.toolbar_gradient_editor_panel(&context);
    }

    fn prototype_workbench_contents_at_width(&mut self, ui: &mut egui::Ui, available_width: f32) {
        let context = self.toolbar_context_kind();
        match presentation_for(self.toolbar_prototype.selected, available_width, context) {
            ResponsivePresentation::Inline => {
                ui.horizontal_centered(|ui| self.workbench_all_controls(ui));
            }
            ResponsivePresentation::MoreMenu => {
                ui.horizontal_centered(|ui| {
                    self.workbench_primary_controls_compact(ui);
                    ui.separator();
                    ui.menu_button("More contextual options…", |ui| {
                        ui.set_min_width(310.0);
                        ui.label(
                            RichText::new(format!("{} OPTIONS", self.tool.label().to_uppercase()))
                                .size(9.0)
                                .strong()
                                .color(SUBTLE),
                        );
                        ui.label(
                            RichText::new(
                                "Every hidden contextual control follows in source focus order.",
                            )
                            .size(9.0)
                            .color(MUTED),
                        );
                        ui.separator();
                        self.workbench_contextual_controls(ui);
                    })
                    .response
                    .on_hover_text(
                        "Open all contextual controls · keyboard: Tab here, then Space or Return",
                    );
                });
            }
            ResponsivePresentation::ScrollRail => {
                ui.horizontal_centered(|ui| {
                    if ui
                        .button("◀")
                        .on_hover_text("Scroll contextual toolbar to the first control")
                        .clicked()
                    {
                        self.toolbar_prototype.scroll_edge = Some(ScrollEdge::Start);
                    }
                    let requested_edge = self.toolbar_prototype.scroll_edge;
                    let rail_width = (ui.available_width() - 34.0).max(160.0);
                    ui.allocate_ui_with_layout(
                        Vec2::new(rail_width, 36.0),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            egui::ScrollArea::horizontal()
                                .id_salt("toolbar-overflow-scroll-rail")
                                .scroll_bar_visibility(
                                    egui::scroll_area::ScrollBarVisibility::AlwaysVisible,
                                )
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        let start = ui.allocate_response(
                                            Vec2::new(1.0, COMPACT_CONTROL_HEIGHT),
                                            egui::Sense::hover(),
                                        );
                                        if requested_edge == Some(ScrollEdge::Start) {
                                            start.scroll_to_me(Some(egui::Align::Min));
                                        }
                                        self.workbench_all_controls(ui);
                                        let end = ui.allocate_response(
                                            Vec2::new(1.0, COMPACT_CONTROL_HEIGHT),
                                            egui::Sense::hover(),
                                        );
                                        if requested_edge == Some(ScrollEdge::End) {
                                            end.scroll_to_me(Some(egui::Align::Max));
                                        }
                                    });
                                });
                        },
                    );
                    self.toolbar_prototype.scroll_edge = None;
                    if ui
                        .button("▶")
                        .on_hover_text("Scroll contextual toolbar to the last control")
                        .clicked()
                    {
                        self.toolbar_prototype.scroll_edge = Some(ScrollEdge::End);
                        ui.ctx().request_repaint();
                    }
                });
            }
            ResponsivePresentation::Wrapped { .. } => {
                ui.horizontal_wrapped(|ui| self.workbench_all_controls(ui));
            }
        }
    }

    pub(super) fn workbench_all_controls(&mut self, ui: &mut egui::Ui) {
        self.workbench_primary_controls(ui);
        self.workbench_contextual_controls(ui);
    }

    pub(super) fn workbench_existing_controls(&mut self, ui: &mut egui::Ui) {
        self.workbench_primary_controls(ui);
        if matches!(self.tool, Tool::Marquee | Tool::Lasso | Tool::MagicWand)
            || self.workspace.document.selection.is_some()
        {
            self.selection_workbench_controls(ui);
        }
        if matches!(self.tool, Tool::Brush | Tool::Eraser) {
            ui.separator();
            self.brush_settings_control(ui);
        }
    }

    pub(super) fn workbench_primary_controls(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("TOOL").size(9.0).strong().color(SUBTLE));
        let label = if self.tool == Tool::Shape {
            format!("Shape · {}", self.shape_kind.label())
        } else {
            self.tool.label().into()
        };
        ui.label(RichText::new(label).size(12.0).strong().color(TEXT));
        if self.tool == Tool::Rotate {
            alternate_shortcut(ui, "R");
        } else {
            shortcut_key(ui, self.tool.shortcut());
        }
        if shortcut_action_button(
            ui,
            WORKBENCH_ACTION_SIZE,
            "Tools & Actions",
            shortcuts::GlobalShortcut::ToolsAndActions.label(),
        )
        .on_hover_text("Search every canvas tool and one-step action")
        .clicked()
        {
            self.tool_palette = Some(PaletteState::default());
        }
        ui.separator();
        ui.label(
            RichText::new(if self.tool == Tool::Shape {
                self.shape_kind.description()
            } else {
                self.tool.description()
            })
            .size(11.0)
            .color(MUTED),
        );
    }

    fn workbench_primary_controls_compact(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("TOOL").size(9.0).strong().color(SUBTLE));
        let label = if self.tool == Tool::Shape {
            format!("Shape · {}", self.shape_kind.label())
        } else {
            self.tool.label().into()
        };
        ui.label(RichText::new(label).size(12.0).strong().color(TEXT))
            .on_hover_text(if self.tool == Tool::Shape {
                self.shape_kind.description()
            } else {
                self.tool.description()
            });
        if self.tool == Tool::Rotate {
            alternate_shortcut(ui, "R");
        } else {
            shortcut_key(ui, self.tool.shortcut());
        }
        if shortcut_action_button(
            ui,
            WORKBENCH_ACTION_SIZE,
            "Tools & Actions",
            shortcuts::GlobalShortcut::ToolsAndActions.label(),
        )
        .on_hover_text("Search every canvas tool and one-step action")
        .clicked()
        {
            self.tool_palette = Some(PaletteState::default());
        }
    }

    pub(super) fn workbench_contextual_controls(&mut self, ui: &mut egui::Ui) {
        let selected_layer = self.selected_layer().cloned();
        let selected_kind = selected_layer.as_ref().map(selected_layer_kind);
        let manifest = control_manifest(
            self.tool,
            selected_kind,
            self.workspace.document.selection.is_some(),
        );
        for control in manifest {
            match control {
                ToolbarControl::ShapeKind => {
                    ui.separator();
                    egui::ComboBox::from_id_salt("toolbar-shape-kind")
                        .selected_text(self.shape_kind.label())
                        .show_ui(ui, |ui| {
                            for shape in ShapeKind::ALL {
                                ui.selectable_value(&mut self.shape_kind, shape, shape.label());
                            }
                        });
                }
                ToolbarControl::GradientEditor => {
                    let layer = selected_layer
                        .as_ref()
                        .expect("gradient manifest requires a selected shape");
                    self.shape_gradient_toolbar_control(ui, layer);
                }
                ToolbarControl::Paragraph => {
                    let layer = selected_layer
                        .as_ref()
                        .expect("paragraph manifest requires selected text");
                    let LayerKind::Text { typography, .. } = &layer.kind else {
                        unreachable!("paragraph manifest requires selected text");
                    };
                    ui.separator();
                    self.paragraph_controls(ui, layer.id, typography);
                }
                ToolbarControl::Selection => self.selection_workbench_controls(ui),
                ToolbarControl::Brush => {
                    ui.separator();
                    self.brush_settings_control(ui);
                }
            }
        }
    }

    fn shape_gradient_toolbar_control(&mut self, ui: &mut egui::Ui, layer: &Layer) {
        let summary = gradient_summary(layer);
        let response = ui.button(summary);
        let panel = GradientEditorPanelState {
            layer_id: layer.id,
            anchor: response.rect.left_bottom(),
            trigger_rect: response.rect,
        };
        if response.clicked() {
            let already_open = self
                .toolbar_prototype
                .gradient_editor_panel
                .is_some_and(|open| open.layer_id == layer.id);
            self.toolbar_prototype.gradient_editor_panel = (!already_open).then_some(panel);
        } else if self
            .toolbar_prototype
            .gradient_editor_panel
            .is_some_and(|open| open.layer_id == layer.id)
        {
            self.toolbar_prototype.gradient_editor_panel = Some(panel);
        }
        response.on_hover_text(
            "Open the complete gradient editor, including each stop's visual color picker",
        );
    }

    fn toolbar_gradient_editor_panel(&mut self, context: &egui::Context) {
        let Some(panel) = self.toolbar_prototype.gradient_editor_panel else {
            return;
        };
        let Some(layer) = self
            .selected_layer()
            .filter(|layer| layer.id == panel.layer_id)
            .cloned()
        else {
            self.toolbar_prototype.gradient_editor_panel = None;
            return;
        };
        let position = gradient_panel_position(panel.anchor, context.content_rect());
        let nested_popup_was_open = egui::Popup::is_any_open(context);
        let surface = egui::Area::new(egui::Id::new((
            "toolbar-gradient-editor-panel",
            panel.layer_id,
        )))
        .order(egui::Order::Foreground)
        .fixed_pos(position)
        .constrain(true)
        .show(context, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_min_width(GRADIENT_POPOVER_WIDTH);
                ui.set_max_width(GRADIENT_POPOVER_WIDTH);
                ui.set_max_height(GRADIENT_POPOVER_HEIGHT);
                ui.label(
                    RichText::new("GRADIENT EDITOR")
                        .size(9.0)
                        .strong()
                        .color(SUBTLE),
                );
                ui.label(
                    RichText::new(
                        "Full visual stop editing · Escape, click away, or toggle to close",
                    )
                    .size(9.0)
                    .color(MUTED),
                );
                ui.separator();
                egui::ScrollArea::vertical()
                    .id_salt(("toolbar-gradient-editor", layer.id))
                    .max_height(GRADIENT_POPOVER_HEIGHT - 54.0)
                    .show(ui, |ui| self.shape_gradient_controls(ui, &layer));
            });
        });
        let escape = context.input(|input| input.key_pressed(egui::Key::Escape));
        let pressed_outside = context.input(|input| {
            input.pointer.any_pressed()
                && input.pointer.interact_pos().is_some_and(|position| {
                    !surface.response.rect.contains(position)
                        && !panel.trigger_rect.contains(position)
                })
        });
        if gradient_panel_should_close(
            escape,
            pressed_outside,
            nested_popup_was_open || egui::Popup::is_any_open(context),
        ) {
            self.toolbar_prototype.gradient_editor_panel = None;
        }
    }

    fn toolbar_context_kind(&self) -> ContextKind {
        let has_selection = self.workspace.document.selection.is_some();
        let selected_kind = self.selected_layer().map(|layer| &layer.kind);
        let selected_shape = selected_kind.is_some_and(|kind| {
            matches!(
                kind,
                LayerKind::Rectangle { .. } | LayerKind::Ellipse { .. }
            )
        });
        let selected_text =
            selected_kind.is_some_and(|kind| matches!(kind, LayerKind::Text { .. }));
        match self.tool {
            Tool::MagicWand => ContextKind::MagicWand,
            Tool::Lasso => ContextKind::Lasso,
            Tool::Marquee => ContextKind::Selection,
            Tool::Shape if has_selection || selected_shape => ContextKind::ShapeWithSelection,
            Tool::Shape => ContextKind::Shape,
            Tool::Text if has_selection || selected_text => ContextKind::TextWithSelection,
            Tool::Text => ContextKind::Text,
            Tool::Brush | Tool::Eraser => ContextKind::Brush,
            _ if has_selection => ContextKind::Selection,
            _ => ContextKind::None,
        }
    }
}

fn gradient_panel_position(anchor: egui::Pos2, viewport: egui::Rect) -> egui::Pos2 {
    let minimum = viewport.min + egui::vec2(GRADIENT_POPOVER_MARGIN, GRADIENT_POPOVER_MARGIN);
    let maximum = egui::pos2(
        (viewport.right() - GRADIENT_POPOVER_WIDTH - GRADIENT_POPOVER_MARGIN).max(minimum.x),
        (viewport.bottom() - GRADIENT_POPOVER_HEIGHT - GRADIENT_POPOVER_MARGIN).max(minimum.y),
    );
    egui::pos2(
        anchor.x.clamp(minimum.x, maximum.x),
        anchor.y.clamp(minimum.y, maximum.y),
    )
}

fn gradient_panel_should_close(
    escape: bool,
    pressed_outside: bool,
    nested_popup_open: bool,
) -> bool {
    (escape || pressed_outside) && !nested_popup_open
}

fn gradient_summary(layer: &Layer) -> String {
    let Some(prism_core::ShapeFill::Gradient(gradient)) = &layer.shape_fill else {
        return "Gradient · Off".into();
    };
    let kind = match gradient.kind {
        prism_core::GradientKind::Linear => "Linear",
        prism_core::GradientKind::Radial => "Radial",
        prism_core::GradientKind::Angle => "Angle",
    };
    format!("Gradient · {kind} · {} stops", gradient.stops.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn review_harness_is_visible_by_default_and_has_an_explicit_off_gate() {
        assert!(prototype_enabled(None));
        assert!(prototype_enabled(Some("A")));
        assert!(prototype_enabled(Some("B")));
        assert!(prototype_enabled(Some("C")));
        for off in ["0", "false", "off", "disabled"] {
            assert!(!prototype_enabled(Some(off)));
        }
    }

    #[test]
    fn actual_control_manifest_covers_every_supported_context_in_source_order() {
        assert_eq!(
            control_manifest(Tool::MagicWand, Some(SelectedLayerKind::Other), false),
            [ToolbarControl::Selection]
        );
        assert_eq!(
            control_manifest(Tool::Shape, Some(SelectedLayerKind::Shape), false),
            [ToolbarControl::ShapeKind, ToolbarControl::GradientEditor]
        );
        assert_eq!(
            control_manifest(Tool::Shape, Some(SelectedLayerKind::Shape), true),
            [
                ToolbarControl::ShapeKind,
                ToolbarControl::GradientEditor,
                ToolbarControl::Selection,
            ]
        );
        assert_eq!(
            control_manifest(Tool::Text, Some(SelectedLayerKind::Text), true),
            [ToolbarControl::Paragraph, ToolbarControl::Selection]
        );
        assert_eq!(
            control_manifest(Tool::Brush, Some(SelectedLayerKind::Other), false),
            [ToolbarControl::Brush]
        );
    }

    #[test]
    fn all_manifest_controls_have_a_reachable_path_at_review_widths() {
        let contexts = [
            (ContextKind::MagicWand, vec![ToolbarControl::Selection]),
            (
                ContextKind::ShapeWithSelection,
                vec![
                    ToolbarControl::ShapeKind,
                    ToolbarControl::GradientEditor,
                    ToolbarControl::Selection,
                ],
            ),
            (
                ContextKind::TextWithSelection,
                vec![ToolbarControl::Paragraph, ToolbarControl::Selection],
            ),
        ];
        for prototype in ToolbarOverflowPrototype::ALL {
            for width in [980.0, 1_200.0, 1_920.0] {
                for (context, manifest) in &contexts {
                    let presentation = presentation_for(prototype, width, *context);
                    let access: Vec<_> = manifest
                        .iter()
                        .map(|control| access_for(presentation, *control))
                        .collect();
                    assert_eq!(access.len(), manifest.len());
                }
            }
        }
    }

    #[test]
    fn layout_matrix_is_stable_at_980_1200_and_1920() {
        for context in [
            ContextKind::Selection,
            ContextKind::Lasso,
            ContextKind::MagicWand,
            ContextKind::ShapeWithSelection,
            ContextKind::TextWithSelection,
        ] {
            for width in [980.0, 1_200.0] {
                assert_eq!(
                    presentation_for(ToolbarOverflowPrototype::TrailingMore, width, context),
                    ResponsivePresentation::MoreMenu
                );
                assert_eq!(
                    presentation_for(ToolbarOverflowPrototype::ScrollRail, width, context),
                    ResponsivePresentation::ScrollRail
                );
                assert_eq!(
                    presentation_for(ToolbarOverflowPrototype::AdaptiveWrap, width, context),
                    ResponsivePresentation::Wrapped {
                        rows: if width < 1_080.0 { 3 } else { 2 }
                    }
                );
            }
        }
        for prototype in ToolbarOverflowPrototype::ALL {
            for context in [
                ContextKind::None,
                ContextKind::Selection,
                ContextKind::MagicWand,
                ContextKind::ShapeWithSelection,
                ContextKind::TextWithSelection,
            ] {
                assert_eq!(
                    presentation_for(prototype, 1_920.0, context),
                    ResponsivePresentation::Inline
                );
            }
        }
    }

    #[test]
    fn forced_width_drives_presentation_without_escaping_the_native_window() {
        assert_eq!(prototype_widths(1_224.0, None), (1_224.0, 1_224.0));
        assert_eq!(prototype_widths(1_224.0, Some(980.0)), (980.0, 980.0));
        assert_eq!(prototype_widths(1_224.0, Some(1_920.0)), (1_224.0, 1_920.0));
    }

    #[test]
    fn gradient_editor_is_always_an_explicit_popover_never_inline_or_clipped() {
        for prototype in ToolbarOverflowPrototype::ALL {
            for width in [980.0, 1_200.0, 1_920.0] {
                let presentation =
                    presentation_for(prototype, width, ContextKind::ShapeWithSelection);
                assert!(matches!(
                    access_for(presentation, ToolbarControl::GradientEditor),
                    ControlAccess::GradientPopover | ControlAccess::MoreMenuThenGradientPopover
                ));
            }
        }
    }

    #[test]
    fn gradient_panel_position_is_bounded_at_every_review_width() {
        for width in [980.0, 1_200.0, 1_920.0] {
            let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 1_080.0));
            for anchor in [
                egui::pos2(-200.0, -200.0),
                egui::pos2(width * 0.5, 40.0),
                egui::pos2(width + 200.0, 1_280.0),
            ] {
                let position = gradient_panel_position(anchor, viewport);
                assert!(position.x >= viewport.left() + GRADIENT_POPOVER_MARGIN);
                assert!(position.y >= viewport.top() + GRADIENT_POPOVER_MARGIN);
                assert!(
                    position.x + GRADIENT_POPOVER_WIDTH
                        <= viewport.right() - GRADIENT_POPOVER_MARGIN
                );
                assert!(
                    position.y + GRADIENT_POPOVER_HEIGHT
                        <= viewport.bottom() - GRADIENT_POPOVER_MARGIN
                );
            }
        }
    }

    #[test]
    fn gradient_panel_dismissal_yields_to_the_nested_visual_picker() {
        assert!(gradient_panel_should_close(true, false, false));
        assert!(gradient_panel_should_close(false, true, false));
        assert!(!gradient_panel_should_close(false, false, false));
        assert!(
            !gradient_panel_should_close(true, false, true),
            "the first Escape belongs to the nested picker"
        );
        assert!(
            !gradient_panel_should_close(false, true, true),
            "an outside click first dismisses the nested picker without losing its editor"
        );
    }

    #[test]
    fn closed_paths_receive_gradient_access_and_open_paths_do_not() {
        use prism_core::{PathAnchor, PathFillRule, PathGeometry};

        let path = |closed| {
            PathGeometry::new(
                20,
                20,
                closed,
                PathFillRule::EvenOdd,
                vec![
                    PathAnchor::corner(2.0, 2.0),
                    PathAnchor::corner(18.0, 2.0),
                    PathAnchor::corner(10.0, 18.0),
                ],
            )
            .unwrap()
        };
        let closed = Layer {
            kind: LayerKind::Path {
                geometry: path(true),
                color: [255; 4],
            },
            ..Default::default()
        };
        let open = Layer {
            kind: LayerKind::Path {
                geometry: path(false),
                color: [255; 4],
            },
            ..Default::default()
        };
        assert_eq!(selected_layer_kind(&closed), SelectedLayerKind::Shape);
        assert_eq!(selected_layer_kind(&open), SelectedLayerKind::Other);
        assert!(
            control_manifest(Tool::Shape, Some(selected_layer_kind(&closed)), false)
                .contains(&ToolbarControl::GradientEditor)
        );
        assert!(
            !control_manifest(Tool::Shape, Some(selected_layer_kind(&open)), false)
                .contains(&ToolbarControl::GradientEditor)
        );
    }

    #[test]
    fn gradient_summary_reports_state_kind_and_stop_count_truthfully() {
        let disabled = Layer::default();
        assert_eq!(gradient_summary(&disabled), "Gradient · Off");
        for (kind, expected) in [
            (prism_core::GradientKind::Linear, "Linear"),
            (prism_core::GradientKind::Radial, "Radial"),
            (prism_core::GradientKind::Angle, "Angle"),
        ] {
            let mut layer = Layer::default();
            let mut gradient = prism_core::ShapeGradient {
                kind,
                ..Default::default()
            };
            gradient
                .stops
                .push(prism_core::GradientStop::new(0.5, [128, 64, 32, 255]));
            layer.shape_fill = Some(prism_core::ShapeFill::Gradient(gradient));
            assert_eq!(
                gradient_summary(&layer),
                format!("Gradient · {expected} · 3 stops")
            );
        }
    }

    #[test]
    fn option_labels_and_keyboard_order_are_stable() {
        assert_eq!(
            ToolbarOverflowPrototype::ALL.map(ToolbarOverflowPrototype::key),
            ["A", "B", "C"]
        );
        assert_eq!(
            ToolbarOverflowPrototype::ALL.map(ToolbarOverflowPrototype::label),
            ["A · Trailing More", "B · Scroll rail", "C · Adaptive wrap"]
        );
    }
}
