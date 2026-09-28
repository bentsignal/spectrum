//! The canvas size dialog: exact width and height in pixels, optionally
//! keeping proportions, with common sizes a click away.
use crate::{theme::*, workspace::Workspace};
use gpui::{prelude::*, *};
use gpui_component::{
    Sizable, WindowExt,
    button::{Button, ButtonVariants},
    dialog::DialogButtonProps,
    input::{Input, InputEvent, InputState},
    menu::{DropdownMenu, PopupMenuItem},
    switch::Switch,
};
use prism_core::Command;

const PRESETS: [(&str, u32, u32); 6] = [
    ("HD, 1920 × 1080", 1920, 1080),
    ("4K, 3840 × 2160", 3840, 2160),
    ("Square, 1080 × 1080", 1080, 1080),
    ("Portrait, 1080 × 1350", 1080, 1350),
    ("Story, 1080 × 1920", 1080, 1920),
    ("Letter, 2550 × 3300", 2550, 3300),
];
/// Largest edge the dialog accepts.
const LIMIT: u32 = 16384;

pub struct CanvasSize {
    width: Entity<InputState>,
    height: Entity<InputState>,
    keep: bool,
    /// Width over height, held while keeping proportions.
    ratio: f32,
    /// Change events still to come from our own edits to the width and
    /// height fields; those must not flow back into the other field.
    echoes: [u8; 2],
    _subscriptions: Vec<Subscription>,
}

fn parse(input: &Entity<InputState>, cx: &App) -> Option<u32> {
    let value = input.read(cx).value().trim().parse::<u32>().ok()?;
    (1..=LIMIT).contains(&value).then_some(value)
}

impl CanvasSize {
    pub fn new(window: &mut Window, cx: &mut Context<Workspace>) -> Entity<Self> {
        let width = cx.new(|cx| InputState::new(window, cx));
        let height = cx.new(|cx| InputState::new(window, cx));
        cx.new(|cx| {
            let follow = |from_width: bool| {
                move |this: &mut Self,
                      _: &Entity<InputState>,
                      event: &InputEvent,
                      window: &mut Window,
                      cx: &mut Context<Self>| {
                    if matches!(event, InputEvent::Change) {
                        this.follow(from_width, window, cx);
                    }
                }
            };
            let _subscriptions = vec![
                cx.subscribe_in(&width, window, follow(true)),
                cx.subscribe_in(&height, window, follow(false)),
            ];
            Self {
                width,
                height,
                keep: true,
                ratio: 1.,
                echoes: [0; 2],
                _subscriptions,
            }
        })
    }

    /// Keeps the other field in proportion when one changes.
    fn follow(&mut self, from_width: bool, window: &mut Window, cx: &mut Context<Self>) {
        let echo = &mut self.echoes[from_width as usize];
        if *echo > 0 {
            *echo -= 1;
            return;
        }
        if !self.keep {
            return;
        }
        let (source, target) = if from_width {
            (&self.width, &self.height)
        } else {
            (&self.height, &self.width)
        };
        let Some(value) = parse(source, cx) else {
            return;
        };
        let other = if from_width {
            value as f32 / self.ratio
        } else {
            value as f32 * self.ratio
        };
        let other = (other.round() as u32).clamp(1, LIMIT).to_string();
        // Setting a field emits a change too; stop once the pair agrees.
        if target.read(cx).value().trim() == other {
            return;
        }
        let target = target.clone();
        self.echoes[!from_width as usize] += 1;
        target.update(cx, |state, cx| state.set_value(other, window, cx));
    }

    fn set(&mut self, width: u32, height: u32, window: &mut Window, cx: &mut Context<Self>) {
        self.ratio = width as f32 / height.max(1) as f32;
        self.echoes = [1, 1];
        self.width.update(cx, |state, cx| {
            state.set_value(width.to_string(), window, cx)
        });
        self.height.update(cx, |state, cx| {
            state.set_value(height.to_string(), window, cx)
        });
        cx.notify();
    }

    fn set_keep(&mut self, keep: bool, cx: &mut Context<Self>) {
        self.keep = keep;
        if let (Some(w), Some(h)) = (parse(&self.width, cx), parse(&self.height, cx)) {
            self.ratio = w as f32 / h as f32;
        }
        cx.notify();
    }
}

impl Workspace {
    /// Opens the size dialog for the open canvas.
    pub fn open_canvas_size(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(canvas) = &self.canvas else {
            return;
        };
        let (width, height) = (canvas.doc.width, canvas.doc.height);
        let size = self.canvas_size.clone();
        size.update(cx, |size, cx| size.set(width, height, window, cx));
        let view = cx.entity();
        window.open_dialog(cx, move |dialog, _, cx| {
            let current = size.read(cx);
            let (presets, keep) = (size.clone(), size.clone());
            let field = |input: &Entity<InputState>, label: &'static str| {
                div()
                    .flex()
                    .flex_col()
                    .gap_1p5()
                    .flex_1()
                    .child(div().text_xs().text_color(rgb(MUTED)).child(label))
                    .child(
                        Input::new(input)
                            .suffix(div().text_xs().text_color(rgb(FAINT)).child("px")),
                    )
            };
            let body = div()
                .flex()
                .flex_col()
                .gap_4()
                .child(
                    div()
                        .flex()
                        .gap_3()
                        .child(field(&current.width, "Width"))
                        .child(field(&current.height, "Height")),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(
                            Switch::new("keep-proportions")
                                .label("Keep proportions")
                                .checked(current.keep)
                                .on_click(move |checked, _, cx| {
                                    keep.update(cx, |size, cx| size.set_keep(*checked, cx))
                                }),
                        )
                        .child(
                            Button::new("size-presets")
                                .ghost()
                                .small()
                                .label("Presets")
                                .dropdown_caret(true)
                                .dropdown_menu(move |menu, _, _| {
                                    PRESETS.iter().fold(menu, |menu, (name, w, h)| {
                                        let (size, w, h) = (presets.clone(), *w, *h);
                                        menu.item(PopupMenuItem::new(*name).on_click(
                                            move |_, window, cx| {
                                                size.update(cx, |size, cx| {
                                                    size.set(w, h, window, cx)
                                                })
                                            },
                                        ))
                                    })
                                }),
                        ),
                );
            let (apply, read) = (view.clone(), size.clone());
            dialog
                .title("Canvas size")
                .w(px(380.))
                .child(body)
                .confirm()
                .button_props(DialogButtonProps::default().ok_text("Resize"))
                .on_ok(move |_, window, cx| {
                    let read = read.read(cx);
                    let (Some(width), Some(height)) =
                        (parse(&read.width, cx), parse(&read.height, cx))
                    else {
                        return false;
                    };
                    apply.update(cx, |this, cx| {
                        let Some(canvas) = &this.canvas else {
                            return;
                        };
                        let background = canvas.doc.background;
                        this.canvas_commands(
                            vec![Command::SetCanvas {
                                width,
                                height,
                                background,
                            }],
                            window,
                            cx,
                        );
                    });
                    true
                })
        });
    }
}
