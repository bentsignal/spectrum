//! Canvas colors: foreground and background colors (new layers take the
//! foreground), and the pickers
//! for text, fills, and the canvas background.
use crate::{
    color_picker::{ColorPicker, Picked, color_well},
    theme::*,
    workspace::Workspace,
};
use gpui::{prelude::*, *};
use prism_core::{Command, LayerKind};

const FORE: [u8; 4] = [255, 255, 255, 255];
const BACK: [u8; 4] = [214, 214, 214, 255];

pub struct Colors {
    pub fore: [u8; 4],
    pub back: [u8; 4],
    fore_picker: Entity<ColorPicker>,
    back_picker: Entity<ColorPicker>,
    text: Entity<ColorPicker>,
    fill: Entity<ColorPicker>,
    canvas: Entity<ColorPicker>,
    _subscriptions: Vec<Subscription>,
}

impl Colors {
    pub fn new(window: &mut Window, cx: &mut Context<Workspace>) -> Self {
        let fore_picker = ColorPicker::new(FORE, window, cx);
        let back_picker = ColorPicker::new(BACK, window, cx);
        let text = ColorPicker::new(FORE, window, cx);
        let fill = ColorPicker::new(BACK, window, cx);
        let canvas = ColorPicker::new(BACK, window, cx);
        let _subscriptions = vec![
            cx.subscribe(&fore_picker, |this, _, Picked(color), cx| {
                this.colors.fore = *color;
                cx.notify();
            }),
            cx.subscribe(&back_picker, |this, _, Picked(color), cx| {
                this.colors.back = *color;
                cx.notify();
            }),
            cx.subscribe_in(&text, window, |this, _, Picked(color), window, cx| {
                this.update_text(Some(*color), window, cx)
            }),
            cx.subscribe_in(&fill, window, |this, _, Picked(color), window, cx| {
                this.update_shape(Some(*color), window, cx)
            }),
            cx.subscribe_in(&canvas, window, |this, _, Picked(color), window, cx| {
                this.set_canvas_background(*color, window, cx)
            }),
        ];
        Self {
            fore: FORE,
            back: BACK,
            fore_picker,
            back_picker,
            text,
            fill,
            canvas,
            _subscriptions,
        }
    }
}

/// A small arrow pointing up and down, drawn in the current text color.
fn swap_arrow() -> impl IntoElement {
    canvas(
        |_, _, _| {},
        |bounds, _, window, _| {
            let c = bounds.center();
            let color = window.text_style().color;
            let mut path = PathBuilder::stroke(px(1.25));
            path.move_to(c + point(px(0.), px(-6.)));
            path.line_to(c + point(px(0.), px(6.)));
            for side in [-1., 1.] {
                path.move_to(c + point(px(-3.), px(side * 3.)));
                path.line_to(c + point(px(0.), px(side * 6.)));
                path.line_to(c + point(px(3.), px(side * 3.)));
            }
            if let Ok(path) = path.build() {
                window.paint_path(path, color);
            }
        },
    )
    .size(px(14.))
}

/// A label with a color well at the right.
fn color_row(label: &'static str, well: impl IntoElement) -> Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .child(div().text_sm().text_color(rgb(MUTED)).child(label))
        .child(well)
}

impl Workspace {
    pub fn set_canvas_background(
        &mut self,
        background: [u8; 4],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(canvas) = &self.canvas {
            let (width, height) = (canvas.doc.width, canvas.doc.height);
            self.canvas_commands(
                vec![Command::SetCanvas {
                    width,
                    height,
                    background,
                }],
                window,
                cx,
            );
        }
    }

    /// Shows the selected layer's and canvas's colors in their pickers.
    pub fn sync_color_pickers(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let layer = self.selected_layer().map(|l| l.kind.clone());
        let background = self.canvas.as_ref().map(|c| c.doc.background);
        match layer {
            Some(LayerKind::Text { color, .. }) => {
                self.colors
                    .text
                    .update(cx, |picker, cx| picker.set(color, window, cx));
            }
            Some(LayerKind::Rectangle { color, .. } | LayerKind::Ellipse { color, .. }) => {
                self.colors
                    .fill
                    .update(cx, |picker, cx| picker.set(color, window, cx));
            }
            _ => {}
        }
        if let Some(background) = background {
            self.colors
                .canvas
                .update(cx, |picker, cx| picker.set(background, window, cx));
        }
    }

    /// Foreground and background, each labeled with its code, and an arrow
    /// at the left that swaps them (X; D resets them).
    pub fn color_pair(&self, cx: &mut Context<Self>) -> Div {
        let row = |id: &'static str, label: &'static str, color: [u8; 4], picker| {
            let [r, g, b, _] = color;
            div()
                .flex()
                .items_center()
                .gap_2p5()
                .child(color_well(id, color, picker))
                .child(div().flex_1().text_sm().child(label))
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(FAINT))
                        .child(format!("#{r:02X}{g:02X}{b:02X}")),
                )
        };
        div()
            .flex()
            .items_center()
            .gap_1()
            .child(
                div()
                    .id("swap-colors")
                    .w(px(22.))
                    .h(px(56.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_md()
                    .text_color(rgb(MUTED))
                    .hover(|el| el.bg(rgb(HOVER)).text_color(rgb(TEXT)))
                    .child(swap_arrow())
                    .tooltip(|window, cx| {
                        gpui_component::tooltip::Tooltip::new("Swap foreground and background (X)")
                            .build(window, cx)
                    })
                    .on_click(cx.listener(|this, _, window, cx| {
                        let (fore, back) = (this.colors.fore, this.colors.back);
                        this.set_default_color(back, false, window, cx);
                        this.set_default_color(fore, true, window, cx);
                    })),
            )
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(row(
                        "fore",
                        "Foreground",
                        self.colors.fore,
                        &self.colors.fore_picker,
                    ))
                    .child(row(
                        "back",
                        "Background",
                        self.colors.back,
                        &self.colors.back_picker,
                    )),
            )
    }

    pub fn text_color_row(&self, color: [u8; 4]) -> Div {
        color_row("Color", color_well("text-color", color, &self.colors.text))
    }

    pub fn fill_color_row(&self, color: [u8; 4]) -> Div {
        color_row("Color", color_well("fill-color", color, &self.colors.fill))
    }

    pub fn background_color_row(&self, color: [u8; 4]) -> Div {
        color_row(
            "Background",
            color_well("canvas-color", color, &self.colors.canvas),
        )
    }
}

impl Workspace {
    /// Sets the foreground color, or the background color, and its well.
    pub fn set_default_color(
        &mut self,
        color: [u8; 4],
        background: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let colors = &mut self.colors;
        let picker = if background {
            colors.back = color;
            colors.back_picker.clone()
        } else {
            colors.fore = color;
            colors.fore_picker.clone()
        };
        picker.update(cx, |picker, cx| picker.set(color, window, cx));
        cx.notify();
    }
}
