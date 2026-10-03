//! Canvas sidebar modes. Layers holds structure: adding, ordering, showing,
//! and removing layers. Style holds the selected layer's appearance, or the
//! canvas's when nothing is selected.
use crate::{
    controls::{Field, chip, group, slider_row},
    theme::*,
    workspace::Workspace,
};
use gpui::{prelude::*, *};
use gpui_component::{
    Icon, IconName, Sizable, WindowExt,
    button::{Button, ButtonVariants},
    input::Input,
};
use prism_core::{BlendMode, Command, LayerKind, TextAlignment};

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

/// A layer row being dragged, and the ghost that follows the pointer.
pub struct LayerRow(pub u64, String);

impl Render for LayerRow {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_3()
            .py_1p5()
            .rounded_md()
            .bg(rgb(SELECTED))
            .shadow_lg()
            .text_sm()
            .child(self.1.clone())
    }
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
        if let LayerKind::Text { typography, .. } = &layer.kind {
            let (line, tracking) = (typography.line_height, typography.tracking);
            self.line_height
                .update(cx, |s, cx| s.set_value(line, window, cx));
            self.tracking
                .update(cx, |s, cx| s.set_value(tracking, window, cx));
        }
        self.text_size
            .update(cx, |s, cx| s.set_value(size.max(8.), window, cx));
        self.corner
            .update(cx, |s, cx| s.set_value(radius, window, cx));
        let rotation = layer.transform.rotation;
        let rotation = if rotation > 180. {
            rotation - 360.
        } else {
            rotation
        };
        self.rotation
            .update(cx, |s, cx| s.set_value(rotation, window, cx));
        self.sync_style_controls(window, cx);
        // Leave the field alone while it is being typed in.
        let typing = self.text_input.focus_handle(cx).is_focused(window);
        if let Some(text) = text.filter(|_| !typing) {
            self.text_input
                .update(cx, |s, cx| s.set_value(text, window, cx));
        }
        self.sync_color_pickers(window, cx);
        if self.mode == crate::workspace::Mode::Color {
            self.load_layer_color(window, cx);
        }
    }

    pub fn layers_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(canvas) = &self.canvas else {
            return div();
        };
        let defaults = self.default_colors(cx).into_any_element();
        let tool = self.tool_button(cx).into_any_element();
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
                // Drag a row to move the layer; the list shows where it lands.
                .on_drag(LayerRow(id, name.clone()), |row, _, _, cx| {
                    cx.new(|_| LayerRow(row.0, row.1.clone()))
                })
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
        let replaceable = self
            .selected_layer()
            .is_some_and(|l| matches!(l.kind, LayerKind::Raster { .. }));
        let action = |id: &'static str, icon: IconName, tip: &'static str| {
            Button::new(id).ghost().small().icon(icon).tooltip(tip)
        };
        div()
            .flex()
            .flex_col()
            .gap_5()
            .child(group("Tool", Some(defaults)).child(tool))
            .child(
                group(
                    "Layers",
                    has_selection.then(|| {
                        div()
                            .flex()
                            .when(replaceable, |el| {
                                el.child(
                                    action("layer-replace", IconName::Replace, "Replace image")
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            if let Some(id) =
                                                this.canvas.as_ref().and_then(|c| c.selected)
                                            {
                                                this.open_replace_picker(id, window, cx)
                                            }
                                        })),
                                )
                            })
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
                .child(self.layer_list(rows.collect(), cx)),
            )
    }

    /// Style's Look section, or the canvas background when nothing is selected.
    pub fn look_section(&self, cx: &mut Context<Self>) -> Div {
        let Some(layer) = self.selected_layer() else {
            let background = self.canvas.as_ref().map(|c| c.doc.background);
            return div().flex().flex_col().gap_6().child(
                group("Canvas", None)
                    .child(self.background_color_row(background.unwrap_or([0, 0, 0, 255]))),
            );
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
            LayerKind::Text {
                color, typography, ..
            } => {
                let size = self.text_size.read(cx).value().start();
                let line = self.line_height.read(cx).value().start();
                let tracking = self.tracking.read(cx).value().start();
                let align = match typography.alignment {
                    TextAlignment::Left => 0,
                    TextAlignment::Center => 1,
                    TextAlignment::Right => 2,
                };
                let view = cx.entity();
                let family = self
                    .canvas
                    .as_ref()
                    .map(|c| crate::font_browser::current_family(&c.doc, typography))
                    .unwrap_or_default();
                let font = self.font_field(family, cx);
                body = body.child(
                    group("Text", None)
                        .gap_4()
                        .child(Input::new(&self.text_input))
                        .child(font)
                        .child(crate::controls::segmented(
                            "text-align",
                            [(None, "Left"), (None, "Center"), (None, "Right")],
                            align,
                            move |index, window, cx| {
                                let alignment = [
                                    TextAlignment::Left,
                                    TextAlignment::Center,
                                    TextAlignment::Right,
                                ][index];
                                view.update(cx, |this, cx| {
                                    this.update_typography(Some(alignment), window, cx)
                                })
                            },
                        ))
                        .child(slider_row("Size", format!("{size:.0}"), &self.text_size))
                        .child(slider_row(
                            "Line height",
                            format!("{line:.2}"),
                            &self.line_height,
                        ))
                        .child(slider_row(
                            "Tracking",
                            format!("{tracking:.0}"),
                            &self.tracking,
                        ))
                        .child(self.text_color_row(*color)),
                );
            }
            LayerKind::Rectangle { color, .. } | LayerKind::Ellipse { color, .. } => {
                let rectangle = matches!(layer.kind, LayerKind::Rectangle { .. });
                let radius = self.corner.read(cx).value().start();
                let gradient = layer.shape_fill.is_some();
                let color = *color;
                body = body.child(
                    group("Fill", None)
                        .gap_4()
                        .child(
                            div()
                                .flex()
                                .gap_1p5()
                                .child(chip("fill-solid", "Solid", !gradient).on_click(
                                    cx.listener(|this, _, window, cx| {
                                        this.set_gradient(
                                            crate::gradient_editor::GradientTarget::Fill,
                                            None,
                                            window,
                                            cx,
                                        )
                                    }),
                                ))
                                .child(chip("fill-gradient", "Gradient", gradient).on_click(
                                    cx.listener(move |this, _, window, cx| {
                                        this.start_fill_gradient(color, window, cx)
                                    }),
                                )),
                        )
                        .map(|el| {
                            if gradient {
                                el.child(self.gradient_editor(
                                    crate::gradient_editor::GradientTarget::Fill,
                                    cx,
                                ))
                            } else {
                                el.child(self.fill_color_row(color))
                            }
                        })
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

    /// Whether the text field differs from the selected text layer.
    pub fn text_edited(&self, cx: &App) -> bool {
        let value = self.text_input.read(cx).value();
        self.selected_layer().is_some_and(
            |layer| matches!(&layer.kind, LayerKind::Text { text, .. } if *text != value.as_ref()),
        )
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

    /// Sends alignment, line height, and tracking for the selected text layer.
    pub fn update_typography(
        &mut self,
        alignment: Option<TextAlignment>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(layer) = self.selected_layer() else {
            return;
        };
        let LayerKind::Text { typography, .. } = &layer.kind else {
            return;
        };
        let typography = prism_core::TextTypography {
            alignment: alignment.unwrap_or(typography.alignment),
            line_height: self.line_height.read(cx).value().start(),
            tracking: self.tracking.read(cx).value().start(),
            ..typography.clone()
        };
        let id = layer.id;
        self.canvas_commands(
            vec![Command::SetTextTypography { id, typography }],
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
