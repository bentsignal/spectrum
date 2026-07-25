use super::*;
use crate::chrome::{
    PaletteState, ShapeKind,
    chrome_shortcut::{WORKBENCH_ACTION_SIZE, shortcut_action_button},
};

const PROTOTYPE_ENV: &str = "PRISM_TOOLBAR_OVERFLOW_PROTOTYPES";
const PROTOTYPE_WIDTH_ENV: &str = "PRISM_TOOLBAR_PROTOTYPE_WIDTH";
const NARROW_WORKBENCH_HEIGHT: f32 = 88.0;
const VERY_NARROW_WORKBENCH_HEIGHT: f32 = 122.0;

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

#[derive(Debug)]
pub(super) struct ToolbarPrototypeState {
    enabled: bool,
    selected: ToolbarOverflowPrototype,
    forced_width: Option<f32>,
    scroll_edge: Option<ScrollEdge>,
}

impl ToolbarPrototypeState {
    pub(super) fn from_environment() -> Self {
        let value = std::env::var(PROTOTYPE_ENV).ok();
        let enabled = value.is_some();
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
        }
    }

    pub(super) fn enabled(&self) -> bool {
        self.enabled
    }
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
                                "Constrain the real toolbar UI for deterministic comparison when native window resize is unavailable",
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
        let available_width = self
            .toolbar_prototype
            .forced_width
            .unwrap_or_else(|| ui.available_width())
            .min(ui.available_width());
        if self.toolbar_prototype.forced_width.is_some() {
            let height = ui.available_height();
            ui.allocate_ui_with_layout(
                Vec2::new(available_width, height),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| self.prototype_workbench_contents_at_width(ui, available_width),
            );
        } else {
            self.prototype_workbench_contents_at_width(ui, available_width);
        }
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
        if self.tool == Tool::Shape {
            ui.separator();
            egui::ComboBox::from_id_salt("toolbar-shape-kind")
                .selected_text(self.shape_kind.label())
                .show_ui(ui, |ui| {
                    for shape in ShapeKind::ALL {
                        ui.selectable_value(&mut self.shape_kind, shape, shape.label());
                    }
                });
            if let Some(layer) = self.selected_layer().cloned()
                && matches!(
                    layer.kind,
                    LayerKind::Rectangle { .. } | LayerKind::Ellipse { .. }
                )
            {
                self.shape_gradient_controls(ui, &layer);
            }
        }
        if self.tool == Tool::Text {
            ui.separator();
            if let Some(layer) = self.selected_layer().cloned()
                && let LayerKind::Text { typography, .. } = &layer.kind
            {
                self.paragraph_controls(ui, layer.id, typography);
            } else {
                ui.label(RichText::new("Select a text layer for paragraph controls").color(MUTED));
            }
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_narrow_strategy_exposes_the_complete_context_manifest() {
        for context in [
            ContextKind::Selection,
            ContextKind::Lasso,
            ContextKind::MagicWand,
            ContextKind::ShapeWithSelection,
            ContextKind::TextWithSelection,
        ] {
            assert_eq!(
                presentation_for(ToolbarOverflowPrototype::TrailingMore, 980.0, context),
                ResponsivePresentation::MoreMenu
            );
            assert_eq!(
                presentation_for(ToolbarOverflowPrototype::ScrollRail, 980.0, context),
                ResponsivePresentation::ScrollRail
            );
            assert_eq!(
                presentation_for(ToolbarOverflowPrototype::AdaptiveWrap, 980.0, context),
                ResponsivePresentation::Wrapped { rows: 3 }
            );
        }
    }

    #[test]
    fn wide_layout_is_identical_inline_presentation_for_every_prototype() {
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
