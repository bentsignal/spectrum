use gpui::{App, Hsla, px, rgb};
use gpui_component::{Theme, ThemeMode};

/// Main content area. The darkest surface, so the work reads first.
pub const BACKGROUND: u32 = 0x0f0f0f;
pub const SIDEBAR: u32 = 0x161616;
/// Fields, cards, and other raised surfaces.
pub const SURFACE: u32 = 0x1f1f1f;
pub const HOVER: u32 = 0x242424;
pub const SELECTED: u32 = 0x2b2b2b;
pub const BORDER: u32 = 0x262626;
pub const TEXT: u32 = 0xececec;
pub const MUTED: u32 = 0x8c8c8c;
pub const FAINT: u32 = 0x5e5e5e;

pub fn apply(cx: &mut App) {
    Theme::change(ThemeMode::Dark, None, cx);
    let theme = Theme::global_mut(cx);
    theme.font_size = px(14.);
    theme.radius = px(8.);
    theme.radius_lg = px(12.);
    theme.shadow = false;
    let gray = |value| -> Hsla { rgb(value).into() };
    theme.background = gray(BACKGROUND);
    theme.foreground = gray(TEXT);
    theme.border = gray(BORDER);
    theme.input = gray(0x2e2e2e);
    theme.primary = gray(0xe6e6e6);
    theme.primary_foreground = gray(0x141414);
    theme.primary_hover = gray(0xffffff);
    theme.primary_active = gray(0xc4c4c4);
    theme.secondary = gray(SURFACE);
    theme.secondary_foreground = gray(TEXT);
    theme.secondary_hover = gray(HOVER);
    theme.secondary_active = gray(SELECTED);
    theme.muted = gray(SURFACE);
    theme.muted_foreground = gray(MUTED);
    theme.accent = gray(SELECTED);
    theme.accent_foreground = gray(TEXT);
    theme.ring = gray(0x6a6a6a);
    theme.caret = gray(TEXT);
    theme.selection = gray(0x444444);
    theme.slider_bar = gray(0xd0d0d0);
    theme.slider_thumb = gray(0xf4f4f4);
    theme.switch = gray(0x333333);
    theme.switch_thumb = gray(0xf4f4f4);
    theme.popover = gray(0x1c1c1c);
    theme.popover_foreground = gray(TEXT);
    theme.overlay = gpui::hsla(0., 0., 0., 0.6);
    theme.list = gray(0x1c1c1c);
    theme.list_hover = gray(HOVER);
    theme.list_active = gray(SELECTED);
    theme.list_active_border = gray(0x4a4a4a);
    theme.scrollbar_thumb = gray(0x3a3a3a);
    theme.scrollbar_thumb_hover = gray(0x555555);
}
