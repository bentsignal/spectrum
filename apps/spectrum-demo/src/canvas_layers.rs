//! Canvas sidebar modes. Layers holds structure: adding, ordering, showing,
//! and removing layers. Style holds the selected layer's appearance, or the
//! canvas's when nothing is selected.
use crate::{
    controls::{Field, group, slider_row},
    theme::*,
    workspace::Workspace,
};
use gpui::{prelude::*, *};
use gpui_component::{
    Icon, IconName, Sizable, WindowExt,
    button::{Button, ButtonVariants},
    input::Input,
};
use prism_core::{BlendMode, Command, LayerKind};

/// Fill and text colors offered as swatches.
const SWATCHES: [[u8; 4]; 12] = [
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

/// "ColorBurn" becomes "Color burn".
fn blend_name(mode: BlendMode) -> SharedString {
    let raw = format!("{mode:?}");
    let mut out = String::new();
    for (i, c) in raw.chars().enumerate() {
        if i > 0 && c.is_uppercase() {
            out.push(' ');
            out.extend(c.to_lowercase());
        } else {
            out.push(c);
        }
    }
    out.into()
}

fn kind_icon(kind: &LayerKind) -> IconName {
    match kind {
        LayerKind::Text { .. } => IconName::ALargeSmall,
        LayerKind::Raster { .. } => IconName::Frame,
        _ => IconName::LayoutDashboard,
    }
}

fn swatch_row(
    id: &'static str,
    current: Option<[u8; 4]>,
    cx: &mut Context<Workspace>,
    pick: fn(&mut Workspace, [u8; 4], &mut Window, &mut Context<Workspace>),
) -> impl IntoElement {
    div()
        .flex()
        .flex_wrap()
        .gap_1p5()
        .children(SWATCHES.iter().enumerate().map(|(index, color)| {
            let selected = current == Some(*color);
            let color = *color;
            div()
                .id((id, index))
                .size(px(22.))
                .rounded_md()
                .border_2()
                .border_color(rgb(if selected { 0xececec } else { 0x2e2e2e }))
                .bg(rgba(u32::from_be_bytes(color)))
                .on_click(cx.listener(move |this, _, window, cx| pick(this, color, window, cx)))
        }))
}

impl Workspace {
    /// Moves Style controls to the selected layer's values.
    pub fn sync_layer_controls(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(layer) = self.selected_layer() else {
            return;
        };
        let opacity = layer.opacity * 100.;
        let (text, size, radius) = match &layer.kind {
            LayerKind::Text {
                text, font_size, ..
            } => (Some(text.clone()), *font_size, 0.),
            LayerKind::Rectangle { corner_radius, .. } => (None, 0., *corner_radius),
            _ => (None, 0., 0.),
        };
        self.opacity
            .update(cx, |s, cx| s.set_value(opacity, window, cx));
        self.text_size
            .update(cx, |s, cx| s.set_value(size.max(8.), window, cx));
        self.corner
            .update(cx, |s, cx| s.set_value(radius, window, cx));
        if let Some(text) = text {
            self.text_input
                .update(cx, |s, cx| s.set_value(text, window, cx));
        }
        if self.mode == crate::workspace::Mode::Color {
            self.load_layer_color(window, cx);
        }
    }

    /// Where new layers go: the middle of the canvas.
    fn canvas_center(&self) -> (f32, f32) {
        self.canvas.as_ref().map_or((0., 0.), |c| {
            (c.doc.width as f32 / 2., c.doc.height as f32 / 2.)
        })
    }

    fn add_layer(&mut self, kind: &str, window: &mut Window, cx: &mut Context<Self>) {
        let (cx_, cy) = self.canvas_center();
        let short = self
            .canvas
            .as_ref()
            .map_or(1000., |c| c.doc.width.min(c.doc.height) as f32);
        let side = (short * 0.4).round().max(8.);
        let command = match kind {
            "text" => Command::AddText {
                text: "Text".into(),
                name: None,
                font_size: (short * 0.12).round().max(12.),
                color: [255, 255, 255, 255],
                x: cx_ - short * 0.15,
                y: cy - short * 0.06,
                shaping: Default::default(),
            },
            "ellipse" => Command::AddEllipse {
                name: None,
                width: side as u32,
                height: side as u32,
                color: [214, 214, 214, 255],
                x: cx_ - side / 2.,
                y: cy - side / 2.,
            },
            _ => Command::AddRectangle {
                name: None,
                width: (side * 1.4) as u32,
                height: side as u32,
                color: [214, 214, 214, 255],
                corner_radius: 0.,
                x: cx_ - side * 0.7,
                y: cy - side / 2.,
            },
        };
        self.canvas_commands(vec![command], window, cx);
    }

    fn move_selected(&mut self, step: isize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(canvas) = &self.canvas else {
            return;
        };
        let Some(id) = canvas.selected else {
            return;
        };
        let Some(index) = canvas.doc.layers.iter().position(|l| l.id == id) else {
            return;
        };
        let target = (index as isize + step).clamp(0, canvas.doc.layers.len() as isize - 1);
        self.canvas_commands(
            vec![Command::MoveLayer {
                id,
                index: target as usize,
            }],
            window,
            cx,
        );
    }

    pub fn layers_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(canvas) = &self.canvas else {
            return div();
        };
        let add = |id: &'static str, label: &'static str| {
            Button::new(id)
                .small()
                .label(label)
                .flex_1()
                .on_click(cx.listener(move |this, _, window, cx| {
                    if id == "image" {
                        this.open_place_picker(window, cx)
                    } else {
                        this.add_layer(id, window, cx)
                    }
                }))
        };
        let rows = canvas.doc.layers.iter().rev().map(|layer| {
            let id = layer.id;
            let selected = canvas.selected == Some(id);
            let visible = layer.visible;
            let name = layer.name.clone();
            div()
                .id(("layer-row", id))
                .h(px(36.))
                .pl_2p5()
                .pr_1()
                .flex()
                .items_center()
                .gap_2p5()
                .rounded_md()
                .when(selected, |el| el.bg(rgb(SELECTED)))
                .when(!selected, |el| el.hover(|el| el.bg(rgb(HOVER))))
                .child(
                    Icon::new(kind_icon(&layer.kind))
                        .small()
                        .text_color(rgb(MUTED)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_sm()
                        .truncate()
                        .text_color(rgb(if visible { TEXT } else { FAINT }))
                        .child(name.clone()),
                )
                .child(
                    Button::new(("visible", id))
                        .ghost()
                        .xsmall()
                        .icon(if visible {
                            IconName::Eye
                        } else {
                            IconName::EyeOff
                        })
                        .on_click(cx.listener(move |this, _, window, cx| {
                            cx.stop_propagation();
                            this.canvas_commands(
                                vec![Command::SetVisibility {
                                    id,
                                    visible: !visible,
                                }],
                                window,
                                cx,
                            );
                        })),
                )
                .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                    if let Some(canvas) = &mut this.canvas {
                        canvas.selected = Some(id);
                    }
                    this.sync_layer_controls(window, cx);
                    if event.click_count() == 2 {
                        this.open_rename_layer(id, name.clone(), window, cx);
                    }
                    cx.notify();
                }))
        });
        let has_selection = canvas.selected.is_some();
        let action = |id: &'static str, icon: IconName, tip: &'static str| {
            Button::new(id).ghost().small().icon(icon).tooltip(tip)
        };
        div()
            .flex()
            .flex_col()
            .gap_5()
            .child(
                group("Add", None).child(
                    div()
                        .flex()
                        .gap_1p5()
                        .child(add("text", "Text"))
                        .child(add("rectangle", "Box"))
                        .child(add("ellipse", "Circle"))
                        .child(add("image", "Image")),
                ),
            )
            .child(
                group(
                    "Layers",
                    has_selection.then(|| {
                        div()
                            .flex()
                            .child(action("layer-up", IconName::ArrowUp, "Move up").on_click(
                                cx.listener(|this, _, window, cx| {
                                    this.move_selected(1, window, cx)
                                }),
                            ))
                            .child(
                                action("layer-down", IconName::ArrowDown, "Move down").on_click(
                                    cx.listener(|this, _, window, cx| {
                                        this.move_selected(-1, window, cx)
                                    }),
                                ),
                            )
                            .child(action("layer-copy", IconName::Copy, "Duplicate").on_click(
                                cx.listener(|this, _, window, cx| {
                                    this.on_selected(window, cx, |id| Command::DuplicateLayer {
                                        id,
                                    })
                                }),
                            ))
                            .child(action("layer-delete", IconName::Delete, "Delete").on_click(
                                cx.listener(|this, _, window, cx| {
                                    this.on_selected(window, cx, |id| Command::RemoveLayer { id })
                                }),
                            ))
                            .into_any_element()
                    }),
                )
                .gap_0p5()
                .when(canvas.doc.layers.is_empty(), |el| {
                    el.child(
                        div()
                            .text_sm()
                            .text_color(rgb(FAINT))
                            .child("Add text, a shape, or an image."),
                    )
                })
                .children(rows),
            )
    }

    pub fn style_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(layer) = self.selected_layer() else {
            let background = self.canvas.as_ref().map(|c| c.doc.background);
            return div()
                .flex()
                .flex_col()
                .gap_6()
                .child(group("Canvas background", None).child(swatch_row(
                    "background",
                    background,
                    cx,
                    |this, color, window, cx| {
                        if let Some(canvas) = &this.canvas {
                            let (width, height) = (canvas.doc.width, canvas.doc.height);
                            this.canvas_commands(
                                vec![Command::SetCanvas {
                                    width,
                                    height,
                                    background: color,
                                }],
                                window,
                                cx,
                            );
                        }
                    },
                )));
        };
        let view = cx.entity();
        let blend = Field::new("blend", blend_name(layer.blend_mode)).options(
            BlendMode::ALL.iter().map(|m| blend_name(*m)).collect(),
            {
                let view = view.clone();
                move |cx| {
                    let mode = view.read(cx).selected_layer().map(|l| l.blend_mode);
                    BlendMode::ALL
                        .iter()
                        .position(|m| Some(*m) == mode)
                        .unwrap_or(0)
                }
            },
            move |index, window, cx| {
                view.update(cx, |this, cx| {
                    this.on_selected(window, cx, |id| Command::SetBlendMode {
                        id,
                        blend_mode: BlendMode::ALL[index],
                    })
                })
            },
        );
        let opacity = self.opacity.read(cx).value().start();
        let mut body = div().flex().flex_col().gap_6().child(
            group("Blending", None)
                .gap_4()
                .child(blend)
                .child(slider_row(
                    "Opacity",
                    format!("{opacity:.0}%"),
                    &self.opacity,
                )),
        );
        match &layer.kind {
            LayerKind::Text { color, .. } => {
                let size = self.text_size.read(cx).value().start();
                body = body.child(
                    group("Text", None)
                        .gap_4()
                        .child(Input::new(&self.text_input))
                        .child(slider_row("Size", format!("{size:.0}"), &self.text_size))
                        .child(swatch_row(
                            "text-color",
                            Some(*color),
                            cx,
                            |this, color, window, cx| this.update_text(Some(color), window, cx),
                        )),
                );
            }
            LayerKind::Rectangle { color, .. } | LayerKind::Ellipse { color, .. } => {
                let rectangle = matches!(layer.kind, LayerKind::Rectangle { .. });
                let radius = self.corner.read(cx).value().start();
                body = body.child(
                    group("Fill", None)
                        .gap_4()
                        .child(swatch_row(
                            "fill",
                            Some(*color),
                            cx,
                            |this, color, window, cx| this.update_shape(Some(color), window, cx),
                        ))
                        .when(rectangle, |el| {
                            el.child(slider_row(
                                "Corner radius",
                                format!("{radius:.0}"),
                                &self.corner,
                            ))
                        }),
                );
            }
            _ => {}
        }
        body
    }

    fn open_rename_layer(
        &mut self,
        id: u64,
        current: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = self.rename_input.clone();
        input.update(cx, |state, cx| state.set_value(current, window, cx));
        let view = cx.entity();
        window.open_dialog(cx, {
            let input = input.clone();
            move |dialog, _, _| {
                let (view, input) = (view.clone(), input.clone());
                dialog
                    .title("Rename layer")
                    .w(px(400.))
                    .child(Input::new(&input))
                    .confirm()
                    .button_props(
                        gpui_component::dialog::DialogButtonProps::default().ok_text("Rename"),
                    )
                    .on_ok(move |_, window, cx| {
                        let name = input.read(cx).value().trim().to_string();
                        if name.is_empty() {
                            return false;
                        }
                        view.update(cx, |this, cx| {
                            this.canvas_commands(
                                vec![Command::RenameLayer { id, name }],
                                window,
                                cx,
                            )
                        });
                        true
                    })
            }
        });
        cx.defer_in(window, move |_, window, cx| {
            input.update(cx, |state, cx| state.focus(window, cx));
        });
    }

    /// Sends the text field, size slider, and an optional new color.
    pub fn update_text(
        &mut self,
        color: Option<[u8; 4]>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(layer) = self.selected_layer() else {
            return;
        };
        let LayerKind::Text { color: current, .. } = &layer.kind else {
            return;
        };
        let (id, color) = (layer.id, color.unwrap_or(*current));
        let text = self.text_input.read(cx).value().to_string();
        let font_size = self.text_size.read(cx).value().start();
        if text.trim().is_empty() {
            return;
        }
        self.canvas_commands(
            vec![Command::UpdateText {
                id,
                text,
                font_size,
                color,
            }],
            window,
            cx,
        );
    }

    /// Sends a shape's color and corner radius.
    pub fn update_shape(
        &mut self,
        color: Option<[u8; 4]>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(layer) = self.selected_layer() else {
            return;
        };
        let id = layer.id;
        let command = match &layer.kind {
            LayerKind::Rectangle {
                width,
                height,
                color: current,
                ..
            } => Command::UpdateRectangle {
                id,
                width: *width,
                height: *height,
                color: color.unwrap_or(*current),
                corner_radius: self.corner.read(cx).value().start(),
            },
            LayerKind::Ellipse {
                width,
                height,
                color: current,
            } => Command::UpdateEllipse {
                id,
                width: *width,
                height: *height,
                color: color.unwrap_or(*current),
            },
            _ => return,
        };
        self.canvas_commands(vec![command], window, cx);
    }
}
