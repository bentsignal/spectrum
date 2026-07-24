use std::collections::BTreeMap;

use super::*;

// Prism's interface is intentionally close to neutral. Accent is reserved for
// focus, selection, and the one primary action in a local context.
pub(super) const INK: Color32 = Color32::from_rgb(11, 12, 15);
pub(super) const WORKSPACE: Color32 = Color32::from_rgb(14, 15, 18);
pub(super) const PANEL: Color32 = Color32::from_rgb(18, 19, 23);
pub(super) const SURFACE: Color32 = Color32::from_rgb(23, 24, 29);
pub(super) const RAISED: Color32 = Color32::from_rgb(29, 30, 36);
pub(super) const HOVER_SURFACE: Color32 = Color32::from_rgb(37, 38, 45);
pub(super) const ACTIVE_SURFACE: Color32 = Color32::from_rgb(42, 40, 49);
pub(super) const SELECTED_SURFACE: Color32 = Color32::from_rgb(34, 32, 40);
pub(super) const BORDER: Color32 = Color32::from_rgb(45, 46, 54);
pub(super) const BORDER_STRONG: Color32 = Color32::from_rgb(64, 64, 74);
pub(super) const TEXT: Color32 = Color32::from_rgb(229, 227, 233);
pub(super) const MUTED: Color32 = Color32::from_rgb(160, 159, 169);
pub(super) const SUBTLE: Color32 = Color32::from_rgb(150, 149, 159);
pub(super) const ACCENT: Color32 = Color32::from_rgb(174, 161, 198);
pub(super) const ACCENT_WARM: Color32 = Color32::from_rgb(201, 170, 116);
pub(super) const DANGER: Color32 = Color32::from_rgb(218, 111, 120);
pub(super) const CANVAS_EDGE: Color32 = Color32::from_rgb(72, 72, 80);
pub(super) const CHECKER_LIGHT: Color32 = Color32::from_rgb(53, 53, 59);
pub(super) const CHECKER_DARK: Color32 = Color32::from_rgb(41, 41, 46);
pub(super) const TERMINAL_SURFACE: Color32 = WORKSPACE;

pub(super) const RADIUS: f32 = 3.0;
pub(super) const CONTROL_HEIGHT: f32 = 28.0;
pub(super) const COMPACT_CONTROL_HEIGHT: f32 = 24.0;
pub(super) const BUTTON_HORIZONTAL_PADDING: f32 = 10.0;
pub(super) const BUTTON_VERTICAL_PADDING: f32 = 5.0;
pub(super) const CONTROL_HORIZONTAL_PADDING: i8 = 10;
pub(super) const CONTROL_VERTICAL_PADDING: i8 = 6;
pub(super) const COMPACT_CONTROL_HORIZONTAL_PADDING: i8 = 8;
pub(super) const COMPACT_CONTROL_VERTICAL_PADDING: i8 = 4;
const _: () = {
    assert!(CONTROL_HEIGHT > COMPACT_CONTROL_HEIGHT);
    assert!(CONTROL_HORIZONTAL_PADDING > COMPACT_CONTROL_HORIZONTAL_PADDING);
    assert!(CONTROL_VERTICAL_PADDING > COMPACT_CONTROL_VERTICAL_PADDING);
    assert!(CONTROL_HORIZONTAL_PADDING - COMPACT_CONTROL_HORIZONTAL_PADDING == 2);
    assert!(CONTROL_VERTICAL_PADDING - COMPACT_CONTROL_VERTICAL_PADDING == 2);
    assert!(BUTTON_HORIZONTAL_PADDING == CONTROL_HORIZONTAL_PADDING as f32);
};
pub(super) const PANEL_PADDING: i8 = 10;
pub(super) const SECTION_GAP: f32 = 12.0;
#[cfg(target_os = "macos")]
pub(super) const TOP_BAR_HEIGHT: f32 = 36.0;
#[cfg(not(target_os = "macos"))]
pub(super) const TOP_BAR_HEIGHT: f32 = 64.0;
pub(super) const WORKBENCH_HEIGHT: f32 = 40.0;
pub(super) const STATUS_HEIGHT: f32 = 24.0;

pub(super) const MODAL_SHADOW: egui::epaint::Shadow = egui::epaint::Shadow {
    offset: [0, 3],
    blur: 14,
    spread: 1,
    color: Color32::from_black_alpha(72),
};

pub(super) const POPOVER_SHADOW: egui::epaint::Shadow = egui::epaint::Shadow {
    offset: [0, 2],
    blur: 10,
    spread: 0,
    color: Color32::from_black_alpha(60),
};

pub(super) fn inspector_group_heading(ui: &mut egui::Ui, label: &str) {
    ui.add_space(12.0);
    ui.separator();
    ui.add_space(4.0);
    ui.label(RichText::new(label).size(9.0).strong().color(SUBTLE));
    ui.add_space(2.0);
}

pub(super) fn install_style(context: &egui::Context) {
    let radius = egui::CornerRadius::same(RADIUS as u8);
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = PANEL;
    visuals.window_fill = PANEL;
    visuals.window_stroke = Stroke::new(1.0, BORDER_STRONG);
    visuals.window_corner_radius = egui::CornerRadius::same(4);
    visuals.window_shadow = MODAL_SHADOW;
    visuals.menu_corner_radius = radius;
    visuals.popup_shadow = POPOVER_SHADOW;
    visuals.extreme_bg_color = INK;
    visuals.text_edit_bg_color = Some(WORKSPACE);
    visuals.faint_bg_color = SURFACE;
    visuals.code_bg_color = SURFACE;
    visuals.selection.bg_fill = ACTIVE_SURFACE;
    visuals.selection.stroke = Stroke::new(1.0, ACCENT);
    visuals.hyperlink_color = ACCENT;
    visuals.warn_fg_color = ACCENT_WARM;
    visuals.error_fg_color = DANGER;
    visuals.weak_text_color = Some(MUTED);
    visuals.override_text_color = Some(TEXT);
    visuals.slider_trailing_fill = true;
    visuals.disabled_alpha = 0.42;

    visuals.widgets.noninteractive.bg_fill = SURFACE;
    visuals.widgets.noninteractive.weak_bg_fill = Color32::TRANSPARENT;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, MUTED);
    visuals.widgets.noninteractive.corner_radius = radius;

    visuals.widgets.inactive.bg_fill = RAISED;
    visuals.widgets.inactive.weak_bg_fill = RAISED;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, BORDER);
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    visuals.widgets.inactive.corner_radius = radius;

    visuals.widgets.hovered.bg_fill = HOVER_SURFACE;
    visuals.widgets.hovered.weak_bg_fill = HOVER_SURFACE;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, BORDER_STRONG);
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, TEXT);
    visuals.widgets.hovered.corner_radius = radius;

    visuals.widgets.active.bg_fill = ACTIVE_SURFACE;
    visuals.widgets.active.weak_bg_fill = ACTIVE_SURFACE;
    visuals.widgets.active.bg_stroke = Stroke::new(1.0, ACCENT);
    visuals.widgets.active.fg_stroke = Stroke::new(1.0, TEXT);
    visuals.widgets.active.corner_radius = radius;

    visuals.widgets.open = visuals.widgets.active;
    context.set_visuals(visuals);
    context.all_styles_mut(|style| {
        style.text_styles = BTreeMap::from([
            (
                egui::TextStyle::Heading,
                FontId::new(17.0, egui::FontFamily::Proportional),
            ),
            (
                egui::TextStyle::Body,
                FontId::new(13.0, egui::FontFamily::Proportional),
            ),
            (
                egui::TextStyle::Button,
                FontId::new(12.0, egui::FontFamily::Proportional),
            ),
            (
                egui::TextStyle::Small,
                FontId::new(10.0, egui::FontFamily::Proportional),
            ),
            (
                egui::TextStyle::Monospace,
                FontId::new(11.0, egui::FontFamily::Monospace),
            ),
        ]);
        style.spacing.item_spacing = Vec2::new(6.0, 6.0);
        style.spacing.button_padding =
            Vec2::new(BUTTON_HORIZONTAL_PADDING, BUTTON_VERTICAL_PADDING);
        style.spacing.interact_size.y = CONTROL_HEIGHT;
        style.spacing.slider_width = 92.0;
        style.spacing.combo_width = 96.0;
        style.interaction.selectable_labels = false;
        style.interaction.interact_radius = 3.0;
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn relative_luminance(color: Color32) -> f32 {
        let linear = |channel: u8| {
            let value = f32::from(channel) / 255.0;
            if value <= 0.04045 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * linear(color.r()) + 0.7152 * linear(color.g()) + 0.0722 * linear(color.b())
    }

    fn contrast_ratio(first: Color32, second: Color32) -> f32 {
        let first = relative_luminance(first);
        let second = relative_luminance(second);
        (first.max(second) + 0.05) / (first.min(second) + 0.05)
    }

    #[test]
    fn surfaces_progress_monotonically_toward_text() {
        fn luminance(color: Color32) -> u16 {
            u16::from(color.r()) + u16::from(color.g()) + u16::from(color.b())
        }
        assert!(luminance(INK) < luminance(PANEL));
        assert!(luminance(PANEL) < luminance(SURFACE));
        assert!(luminance(SURFACE) < luminance(RAISED));
        assert!(luminance(RAISED) < luminance(HOVER_SURFACE));
        assert!(luminance(MUTED) < luminance(TEXT));
    }

    #[test]
    fn accent_is_restrained_but_distinct_from_neutral_surfaces() {
        let channel_span =
            ACCENT.r().max(ACCENT.g()).max(ACCENT.b()) - ACCENT.r().min(ACCENT.g()).min(ACCENT.b());
        assert!(channel_span <= 48);
        assert_ne!(ACCENT, BORDER_STRONG);
    }

    #[test]
    fn chrome_reserves_less_space_than_the_previous_baseline() {
        const PREVIOUS_CHROME_HEIGHT: f32 = 82.0 + 52.0 + 30.0;
        let chrome_height = TOP_BAR_HEIGHT + WORKBENCH_HEIGHT + STATUS_HEIGHT;
        #[cfg(target_os = "macos")]
        assert_eq!(chrome_height, 100.0);
        #[cfg(not(target_os = "macos"))]
        assert_eq!(chrome_height, 128.0);
        assert!(chrome_height <= PREVIOUS_CHROME_HEIGHT - 36.0);
    }

    #[test]
    fn text_and_focus_tokens_clear_dark_surface_contrast_targets() {
        assert!(contrast_ratio(TEXT, PANEL) >= 7.0);
        assert!(contrast_ratio(MUTED, PANEL) >= 4.5);
        assert!(contrast_ratio(MUTED, ACTIVE_SURFACE) >= 4.5);
        assert!(contrast_ratio(SUBTLE, PANEL) >= 4.5);
        assert!(contrast_ratio(SUBTLE, ACTIVE_SURFACE) >= 4.5);
        assert!(contrast_ratio(SUBTLE, SELECTED_SURFACE) >= 4.5);
        assert!(contrast_ratio(ACCENT, PANEL) >= 3.0);
        assert!(contrast_ratio(DANGER, PANEL) >= 4.5);
    }

    #[test]
    fn modal_and_popover_shadows_are_restrained_and_fit_representative_surfaces() {
        for shadow in [MODAL_SHADOW, POPOVER_SHADOW] {
            let margin = shadow.margin();
            assert!(margin.left >= 0.0);
            assert!(margin.top >= 0.0);
            assert!(margin.right <= 12.0);
            assert!(margin.bottom <= 12.0);
            assert!(shadow.color.a() <= 72);
        }

        for size in [Vec2::new(360.0, 108.0), Vec2::new(520.0, 454.0)] {
            let rect = Rect::from_min_size(Pos2::new(24.0, 24.0), size);
            let shadow_bounds = rect + MODAL_SHADOW.margin();
            assert!(shadow_bounds.width() < size.x + 24.0);
            assert!(shadow_bounds.height() < size.y + 24.0);
        }
    }

    #[test]
    fn shadow_alpha_supports_dark_and_light_surface_contrast_without_a_halo() {
        let composite = |background: Color32, shadow: egui::epaint::Shadow| {
            let alpha = f32::from(shadow.color.a()) / 255.0;
            let channel = |value: u8| ((f32::from(value) * (1.0 - alpha)).round()) as u8;
            Color32::from_rgb(
                channel(background.r()),
                channel(background.g()),
                channel(background.b()),
            )
        };
        for background in [PANEL, Color32::from_rgb(242, 242, 245)] {
            let modal = composite(background, MODAL_SHADOW);
            let popover = composite(background, POPOVER_SHADOW);
            assert_ne!(modal, background);
            assert_ne!(popover, background);
            assert!(contrast_ratio(modal, background) < 2.0);
            assert!(contrast_ratio(popover, background) < 2.0);
        }
    }
}
