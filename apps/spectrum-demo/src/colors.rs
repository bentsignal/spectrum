//! Canvas colors: foreground and background defaults for new layers (text
//! takes the foreground, boxes and circles the background), and the pickers
//! for text, fills, and the canvas background.
use crate::{
    color_picker::{ColorPicker, Picked, color_well},
    theme::*,
    workspace::Workspace,
};
use gpui::{prelude::*, *};
use gpui_component::{
    IconName, Sizable,
    button::{Button, ButtonVariants},
};
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
            .child(color_well(
                "back",
                self.colors.back,
                &self.colors.back_picker,
            ))
            .child(
                Button::new("swap-colors")
                    .ghost()
                    .xsmall()
                    .icon(IconName::Replace)
                    .tooltip("Swap foreground and background")
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
