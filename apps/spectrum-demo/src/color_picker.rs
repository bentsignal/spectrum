//! A full color picker: a saturation and brightness field, a hue strip, the
//! color as text (hex, RGB, HSL, or OKLCH), and common colors. Color wells open it in a popover.
use crate::{
    color_text::{self, FORMATS, Format},
    theme::*,
};
use gpui::{prelude::*, *};
use gpui_component::{
    IconName, Selectable, Sizable,
    button::{Button, ButtonVariants},
    input::{Input, InputEvent, InputState},
    popover::Popover,
};
use std::{cell::RefCell, rc::Rc};

/// Common colors under the picker.
pub const SWATCHES: [[u8; 4]; 12] = [
    [255, 255, 255, 255],
    [214, 214, 214, 255],
    [128, 128, 128, 255],
    [52, 52, 52, 255],
    [16, 16, 16, 255],
    [229, 72, 77, 255],
    [240, 136, 62, 255],
    [229, 195, 75, 255],
    [76, 181, 113, 255],
    [63, 184, 176, 255],
    [74, 127, 224, 255],
    [142, 90, 214, 255],
];
const WIDTH: f32 = 256.;

/// A color was chosen, as RGBA.
pub struct Picked(pub [u8; 4]);

#[derive(Clone, Copy, PartialEq)]
enum Part {
    Field,
    Strip,
}

pub struct ColorPicker {
    /// Hue in degrees, saturation and value from 0 to 1.
    hsv: (f32, f32, f32),
    alpha: u8,
    hex: Entity<InputState>,
    format: Format,
    /// The copy button shows a check briefly after copying.
    copied: bool,
    /// The format list is open.
    choosing: bool,
    field: Rc<RefCell<Bounds<Pixels>>>,
    strip: Rc<RefCell<Bounds<Pixels>>>,
    drag: Option<Part>,
    /// Text field changes we made ourselves, which must not re-pick.
    echoes: u8,
    _subscription: Subscription,
}

impl EventEmitter<Picked> for ColorPicker {}

pub fn to_hsv([r, g, b, _]: [u8; 4]) -> (f32, f32, f32) {
    let (r, g, b) = (r as f32 / 255., g as f32 / 255., b as f32 / 255.);
    let max = r.max(g).max(b);
    let delta = max - r.min(g).min(b);
    let hue = if delta == 0. {
        0.
    } else if max == r {
        60. * ((g - b) / delta).rem_euclid(6.)
    } else if max == g {
        60. * ((b - r) / delta + 2.)
    } else {
        60. * ((r - g) / delta + 4.)
    };
    let saturation = if max == 0. { 0. } else { delta / max };
    (hue, saturation, max)
}

/// The pure-hue part of an RGB color with chroma `c`, before lightness.
pub fn hue_rgb(h: f32, c: f32) -> (f32, f32, f32) {
    let x = c * (1. - ((h / 60.).rem_euclid(2.) - 1.).abs());
    match (h.rem_euclid(360.) / 60.) as u32 {
        0 => (c, x, 0.),
        1 => (x, c, 0.),
        2 => (0., c, x),
        3 => (0., x, c),
        4 => (x, 0., c),
        _ => (c, 0., x),
    }
}

pub fn to_rgb((h, s, v): (f32, f32, f32)) -> [u8; 3] {
    let c = v * s;
    let (r, g, b) = hue_rgb(h, c);
    let m = v - c;
    [r, g, b].map(|channel| ((channel + m) * 255.).round() as u8)
}

pub fn to_hsla([r, g, b, a]: [u8; 4]) -> Hsla {
    rgba(u32::from_be_bytes([r, g, b, a])).into()
}

impl ColorPicker {
    pub fn new(color: [u8; 4], window: &mut Window, cx: &mut App) -> Entity<Self> {
        let hex = cx.new(|cx| InputState::new(window, cx));
        cx.new(|cx| {
            let _subscription = cx.subscribe_in(
                &hex,
                window,
                |this: &mut Self, _, event: &InputEvent, window, cx| {
                    if !matches!(event, InputEvent::Change | InputEvent::PressEnter { .. }) {
                        return;
                    }
                    if this.echoes > 0 {
                        this.echoes -= 1;
                        return;
                    }
                    let text = this.hex.read(cx).value().to_string();
                    if let Some(color) = color_text::parse(&text, this.format) {
                        this.set_rgba(color);
                        this.pick(window, cx, false);
                    }
                },
            );
            let mut picker = Self {
                hsv: to_hsv(color),
                alpha: color[3],
                hex,
                format: color_text::preferred(),
                copied: false,
                choosing: false,
                field: Default::default(),
                strip: Default::default(),
                drag: None,
                echoes: 0,
                _subscription,
            };
            picker.show_hex(window, cx);
            picker
        })
    }

    pub fn rgba(&self) -> [u8; 4] {
        let [r, g, b] = to_rgb(self.hsv);
        [r, g, b, self.alpha]
    }

    fn set_rgba(&mut self, color: [u8; 4]) {
        let (hue, saturation, value) = to_hsv(color);
        // Grays have no hue; keep the one the user was on.
        let hue = if saturation == 0. { self.hsv.0 } else { hue };
        self.hsv = (hue, saturation, value);
        self.alpha = color[3];
    }

    /// Shows `color` without reporting it as picked.
    pub fn set(&mut self, color: [u8; 4], window: &mut Window, cx: &mut Context<Self>) {
        if color == self.rgba() {
            return;
        }
        self.set_rgba(color);
        self.show_hex(window, cx);
        cx.notify();
    }

    fn show_hex(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = color_text::format(self.rgba(), self.format);
        self.echoes += 1;
        self.hex
            .update(cx, |state, cx| state.set_value(text, window, cx));
    }

    fn pick(&mut self, window: &mut Window, cx: &mut Context<Self>, update_hex: bool) {
        if update_hex {
            self.show_hex(window, cx);
        }
        cx.emit(Picked(self.rgba()));
        cx.notify();
    }

    fn point_at(
        &mut self,
        part: Part,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let bounds = *match part {
            Part::Field => &self.field,
            Part::Strip => &self.strip,
        }
        .borrow();
        let x = (f32::from(position.x - bounds.origin.x) / f32::from(bounds.size.width).max(1.))
            .clamp(0., 1.);
        let y = (f32::from(position.y - bounds.origin.y) / f32::from(bounds.size.height).max(1.))
            .clamp(0., 1.);
        match part {
            Part::Field => {
                self.hsv.1 = x;
                self.hsv.2 = 1. - y;
            }
            Part::Strip => self.hsv.0 = x * 359.9,
        }
        self.pick(window, cx, true);
    }

    fn press(
        &mut self,
        part: Part,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.drag = Some(part);
        self.point_at(part, event.position, window, cx);
    }
}

/// A slot that records where it was laid out, for mapping the pointer.
fn measured(slot: &Rc<RefCell<Bounds<Pixels>>>) -> impl IntoElement {
    let slot = slot.clone();
    canvas(
        move |bounds, _, _| *slot.borrow_mut() = bounds,
        |_, _, _, _| {},
    )
    .absolute()
    .size_full()
}

impl Render for ColorPicker {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (hue, saturation, value) = self.hsv;
        let pure = hsla(hue / 360., 1., 0.5, 1.);
        let current = to_hsla(self.rgba());
        // Another picker may have changed the shared format.
        if self.format != color_text::preferred() {
            self.format = color_text::preferred();
            self.show_hex(window, cx);
        }
        // A list inside the picker; a menu cannot open from a popover.
        let format_menu = Button::new("color-format")
            .ghost()
            .xsmall()
            .label(color_text::name(self.format))
            .dropdown_caret(true)
            .selected(self.choosing)
            .on_click(cx.listener(|this, _, _, cx| {
                this.choosing = !this.choosing;
                cx.notify();
            }));
        let format_list = self.choosing.then(|| {
            div()
                .absolute()
                .bottom(px(84.))
                .left(px(30.))
                .w(px(120.))
                .p_1()
                .flex()
                .flex_col()
                .rounded_md()
                .border_1()
                .border_color(rgb(BORDER))
                .bg(rgb(0x1c1c1c))
                .shadow_lg()
                // Clicks stop here instead of picking a color underneath.
                .occlude()
                .children(FORMATS.iter().map(|(format, name)| {
                    let format = *format;
                    div()
                        .id(*name)
                        .px_2()
                        .py_1()
                        .rounded_sm()
                        .text_sm()
                        .flex()
                        .justify_between()
                        .hover(|el| el.bg(rgb(HOVER)))
                        .child(*name)
                        .when(format == self.format, |el| {
                            el.child(div().text_color(rgb(MUTED)).child("✓"))
                        })
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _, window, cx| {
                                cx.stop_propagation();
                                color_text::set_preferred(format);
                                crate::prefs::save_color_format();
                                this.choosing = false;
                                this.format = format;
                                this.show_hex(window, cx);
                                cx.notify();
                            }),
                        )
                }))
        });
        let copy = Button::new("copy-color")
            .ghost()
            .xsmall()
            .icon(if self.copied {
                IconName::Check
            } else {
                IconName::Copy
            })
            .tooltip("Copy")
            .on_click(cx.listener(|this, _, window, cx| {
                let text = color_text::format(this.rgba(), this.format);
                cx.write_to_clipboard(ClipboardItem::new_string(text));
                this.copied = true;
                cx.notify();
                cx.spawn_in(window, async move |this, cx| {
                    cx.background_executor()
                        .timer(std::time::Duration::from_millis(1200))
                        .await;
                    this.update(cx, |this, cx| {
                        this.copied = false;
                        cx.notify();
                    })
                    .ok();
                })
                .detach();
            }));
        let paste = Button::new("paste-color")
            .ghost()
            .xsmall()
            .child(clipboard_glyph())
            .tooltip("Paste")
            .on_click(cx.listener(|this, _, window, cx| {
                let text = cx.read_from_clipboard().and_then(|item| item.text());
                if let Some(color) = text.and_then(|t| color_text::parse(&t, this.format)) {
                    this.set_rgba(color);
                    this.pick(window, cx, true);
                }
            }));
        // While dragging, follow the pointer anywhere in the window.
        if let Some(part) = self.drag {
            let view = cx.entity();
            let moving = view.clone();
            window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                if phase == DispatchPhase::Bubble && event.pressed_button == Some(MouseButton::Left)
                {
                    moving.update(cx, |this, cx| {
                        this.point_at(part, event.position, window, cx)
                    });
                }
            });
            window.on_mouse_event(move |_: &MouseUpEvent, phase, _, cx| {
                if phase == DispatchPhase::Bubble {
                    view.update(cx, |this, cx| {
                        this.drag = None;
                        cx.notify();
                    });
                }
            });
        }
        let field = div()
            .id("color-field")
            .relative()
            .w(px(WIDTH))
            .h(px(148.))
            .rounded_md()
            .overflow_hidden()
            .bg(pure)
            .child(div().absolute().size_full().bg(linear_gradient(
                90.,
                linear_color_stop(hsla(0., 0., 1., 1.), 0.),
                linear_color_stop(hsla(0., 0., 1., 0.), 1.),
            )))
            .child(div().absolute().size_full().bg(linear_gradient(
                180.,
                linear_color_stop(hsla(0., 0., 0., 0.), 0.),
                linear_color_stop(hsla(0., 0., 0., 1.), 1.),
            )))
            .child(measured(&self.field))
            .child(
                div()
                    .absolute()
                    .left(px(saturation * WIDTH - 7.))
                    .top(px((1. - value) * 148. - 7.))
                    .size(px(14.))
                    .rounded_full()
                    .border_2()
                    .border_color(hsla(0., 0., 1., 1.))
                    .shadow_sm(),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event, window, cx| this.press(Part::Field, event, window, cx)),
            );
        // One column per pixel, so the hue runs smoothly without seams.
        let hues = canvas(
            |_, _, _| {},
            |bounds, _, window, _| {
                let width = f32::from(bounds.size.width).max(1.);
                for column in 0..width.ceil() as u32 {
                    let x = column as f32;
                    window.paint_quad(fill(
                        Bounds::new(
                            bounds.origin + point(px(x), px(0.)),
                            size(px(1.), bounds.size.height),
                        ),
                        hsla(x / width, 1., 0.5, 1.),
                    ));
                }
            },
        )
        .size_full();
        let strip = div()
            .id("hue-strip")
            .relative()
            .w(px(WIDTH))
            .h(px(12.))
            .child(
                div()
                    .absolute()
                    .size_full()
                    .rounded_full()
                    .overflow_hidden()
                    .child(hues),
            )
            .child(measured(&self.strip))
            .child(
                div()
                    .absolute()
                    .left(px(hue / 360. * WIDTH - 7.))
                    .top(px(-1.))
                    .size(px(14.))
                    .rounded_full()
                    .border_2()
                    .border_color(hsla(0., 0., 1., 1.))
                    .bg(pure),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event, window, cx| this.press(Part::Strip, event, window, cx)),
            );
        let swatches = SWATCHES.iter().enumerate().map(|(index, color)| {
            let color = *color;
            div()
                .id(("picker-swatch", index))
                .size(px(16.))
                .rounded_sm()
                .border_1()
                .border_color(rgb(0x3a3a3a))
                .bg(to_hsla(color))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.set_rgba(color);
                    this.pick(window, cx, true);
                }))
        });
        div()
            .relative()
            .w(px(WIDTH))
            .flex()
            .flex_col()
            .gap_3()
            .child(field)
            .child(strip)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .size(px(24.))
                            .flex_none()
                            .rounded_md()
                            .border_1()
                            .border_color(rgb(BORDER))
                            .bg(current),
                    )
                    .child(format_menu)
                    .child(div().flex_1())
                    .child(copy)
                    .child(paste),
            )
            // The value on its own line, so every format fits.
            .child(Input::new(&self.hex).small())
            .child(div().flex().justify_between().children(swatches))
            .children(format_list)
    }
}

/// A clipboard mark for the paste button.
fn clipboard_glyph() -> impl IntoElement {
    div()
        .relative()
        .w(px(11.))
        .h(px(13.))
        .rounded_xs()
        .border_1()
        .border_color(rgb(MUTED))
        .child(
            div()
                .absolute()
                .top(px(-2.))
                .left(px(2.))
                .w(px(5.))
                .h(px(3.))
                .rounded_xs()
                .bg(rgb(MUTED)),
        )
}

/// A swatch button that opens `picker` in a popover.
pub fn color_well(id: &'static str, color: [u8; 4], picker: &Entity<ColorPicker>) -> Popover {
    let picker = picker.clone();
    Popover::new(id)
        .trigger(
            Button::new(id)
                .ghost()
                .p_0()
                .child(
                    div()
                        .size(px(24.))
                        .rounded_md()
                        .border_1()
                        .border_color(rgb(0x3a3a3a))
                        .bg(to_hsla(color)),
                )
                .selected(false),
        )
        .content(move |_, _, _| picker.clone())
}
