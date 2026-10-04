//! The Tool tab of a canvas's sidebar: the current tool and the foreground
//! and background colors, the tool's settings (brush size, hardness, and
//! opacity; wand tolerance), what a selection can do, and how to use the
//! tool. The Layers tab keeps only the layers.
use crate::{
    controls::{chip, group, slider_row, toggle},
    theme::*,
    tools::Tool,
    workspace::Workspace,
};
use gpui::{prelude::*, *};

/// How to use each tool, in a line.
fn hint(tool: Tool) -> &'static str {
    match tool {
        Tool::Move => {
            "Drag to move. Corners resize; the handle on top rotates (Shift turns in 15° steps)."
        }
        Tool::Marquee | Tool::EllipseSelect | Tool::Lasso => {
            "Drag to select. Shift-drag adds, Option-drag subtracts, both intersect. Click off the canvas or press Esc to deselect."
        }
        Tool::Wand => {
            "Click a color to select it. Shift-click adds, Option-click subtracts. Click off the canvas or press Esc to deselect."
        }
        Tool::Brush => {
            "Paints the foreground color on a Paint layer. A selection limits where paint goes. [ and ] change the size."
        }
        Tool::Eraser => "Erases whatever layer you start on, of any kind. [ and ] change the size.",
        Tool::Text => "Click to place text.",
        Tool::Box | Tool::Circle => {
            "Drag to draw in the foreground color; it stays square. Shift-drag for any proportions."
        }
        Tool::Gradient => "Drag from where the gradient starts to where it ends.",
        Tool::Pen => {
            "Click for corners, drag for curves. Shift keeps 45°. Enter or the first point closes; Esc cancels."
        }
        Tool::Crop => "Drag the area to keep.",
        Tool::Eyedropper => {
            "Click to take a color as the foreground; Option-click for the background. ⌘C copies the code under the pointer."
        }
        Tool::Image => "Place an image from the project.",
    }
}

impl Workspace {
    pub fn tool_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(canvas) = &self.canvas else {
            return div();
        };
        let tool = canvas.tool;
        let selected = canvas.doc.selection.is_some();
        let defaults = self.default_colors(cx).into_any_element();
        let mut panel = div().flex().flex_col().gap_6().child(
            group("Tool", Some(defaults))
                .gap_3()
                .child(self.tool_button(cx))
                .child(div().text_xs().text_color(rgb(FAINT)).child(hint(tool))),
        );
        match tool {
            Tool::Brush | Tool::Eraser => {
                let (size, hardness, opacity) = self.tool_options.brush(cx);
                let options = &self.tool_options;
                panel = panel.child(
                    group(
                        if tool == Tool::Brush {
                            "Brush"
                        } else {
                            "Eraser"
                        },
                        None,
                    )
                    .gap_3()
                    .child(slider_row(
                        "Size",
                        format!("{size:.0}"),
                        &options.brush_size,
                    ))
                    .child(slider_row(
                        "Hardness",
                        format!("{:.0}%", hardness * 100.),
                        &options.brush_hardness,
                    ))
                    .child(slider_row(
                        "Opacity",
                        format!("{:.0}%", opacity * 100.),
                        &options.brush_opacity,
                    )),
                );
            }
            Tool::Wand => {
                let tolerance = self.tool_options.tolerance(cx);
                panel = panel.child(
                    group("Magic wand", None)
                        .gap_3()
                        .child(slider_row(
                            "Tolerance",
                            tolerance.to_string(),
                            &self.tool_options.wand_tolerance,
                        ))
                        .child(
                            toggle(
                                "wand-contiguous",
                                "Contiguous",
                                self.tool_options.contiguous,
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.tool_options.contiguous = !this.tool_options.contiguous;
                                cx.notify();
                            })),
                        ),
                );
            }
            _ => {}
        }
        if selected || tool.selects() {
            let action =
                |id: &'static str,
                 label: &'static str,
                 run: fn(&mut Self, &mut Window, &mut Context<Self>)| {
                    chip(id, label, false)
                        .on_click(cx.listener(move |this, _, window, cx| run(this, window, cx)))
                };
            let mut actions = group("Selection", None).gap_1p5().child(
                div()
                    .flex()
                    .gap_1p5()
                    .child(action("select-all", "All", Self::select_canvas))
                    .child(action("select-none", "None", Self::deselect))
                    .child(action("select-invert", "Invert", Self::invert_selection)),
            );
            if selected {
                actions = actions.child(
                    div()
                        .flex()
                        .gap_1p5()
                        .child(action("selection-fill", "Fill", Self::fill_selection))
                        .child(action(
                            "selection-delete",
                            "Hide",
                            Self::delete_in_selection,
                        ))
                        .child(action("selection-crop", "Crop", Self::crop_to_selection)),
                );
            }
            panel = panel.child(actions);
        }
        panel
    }
}
