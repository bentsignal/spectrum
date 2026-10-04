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

/// A small arrow pointing both ways, drawn in the current text color.
fn swap_arrow() -> impl IntoElement {
    canvas(
        |_, _, _| {},
        |bounds, _, window, _| {
            let c = bounds.center();
            let color = window.text_style().color;
            let mut path = PathBuilder::stroke(px(1.25));
            path.move_to(c + point(px(-6.), px(0.)));
            path.line_to(c + point(px(6.), px(0.)));
            for side in [-1., 1.] {
                path.move_to(c + point(px(side * 3.), px(-3.)));
                path.line_to(c + point(px(side * 6.), px(0.)));
                path.line_to(c + point(px(side * 3.), px(3.)));
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

    /// Foreground and background wells, with a swap button.
    pub fn default_colors(&self, cx: &mut Context<Self>) -> Div {
        div()
            .flex()
            .items_center()
            .gap_2()
            .child(color_well(
                "fore",
                self.colors.fore,
                &self.colors.fore_picker,
            ))
            .child(
                div()
                    .id("swap-colors")
                    .w(px(18.))
                    .h(px(24.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_sm()
                    .text_color(rgb(MUTED))
                    .hover(|el| el.text_color(rgb(TEXT)))
                    .child(swap_arrow())
                    .tooltip(|window, cx| {
                        gpui_component::tooltip::Tooltip::new("Swap colors").build(window, cx)
                    })
                    .on_click(cx.listener(|this, _, window, cx| {
                        let colors = &mut this.colors;
                        (colors.fore, colors.back) = (colors.back, colors.fore);
                        let (fore, back) = (colors.fore, colors.back);
                        colors
                            .fore_picker
                            .update(cx, |picker, cx| picker.set(fore, window, cx));
                        colors
                            .back_picker
                            .update(cx, |picker, cx| picker.set(back, window, cx));
                        cx.notify();
                    })),
            )
            .child(color_well(
                "back",
                self.colors.back,
                &self.colors.back_picker,
            ))
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
