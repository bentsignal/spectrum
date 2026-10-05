//! A canvas's Overview, where its sidebar starts: the current tool and its
//! settings, the selected layer (a click opens Layers), the foreground and
//! background colors, and what a selection can do.
use crate::{
    controls::{chip, group, slider_row, toggle},
    theme::*,
    tools::Tool,
    workspace::Workspace,
};
use gpui::{prelude::*, *};
use gpui_component::{Icon, IconName, Sizable};

impl Workspace {
    pub fn tool_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(canvas) = &self.canvas else {
            return div();
        };
        let tool = canvas.tool;
        let selected = canvas.doc.selection.is_some();
        let layer = canvas.selected.and_then(|id| canvas.doc.layer(id).ok());
        let current = div()
            .id("overview-layer")
            .h(px(40.))
            .px_3()
            .flex()
            .items_center()
            .gap_2p5()
            .rounded_md()
            .border_1()
            .border_color(rgb(BORDER))
            .bg(rgb(SURFACE))
            .hover(|el| el.bg(rgb(HOVER)))
            .map(|el| match layer {
                Some(layer) => el
                    .child(
                        Icon::new(crate::canvas_layers::kind_icon(&layer.kind))
                            .small()
                            .text_color(rgb(MUTED)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_sm()
                            .child(layer.name.clone()),
                    ),
                None => el.child(
                    div()
                        .flex_1()
                        .text_sm()
                        .text_color(rgb(FAINT))
                        .child("No layer selected"),
                ),
            })
            .child(
                Icon::new(IconName::ChevronRight)
                    .small()
                    .text_color(rgb(FAINT)),
            )
            .on_click(cx.listener(|this, _, window, cx| {
                this.set_mode(crate::workspace::Mode::Layers, window, cx)
            }));
        // The tool first, where every mode has it.
        let mut panel = div().flex().flex_col().gap_6().child(self.tool_button(cx));
        let settings = self.tool_settings(tool, cx);
        panel = panel
            .children(settings)
            .child(group("Selected layer", None).child(current))
            .child(group("Colors", None).child(self.color_pair(cx)));
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

    /// The current tool's own settings, if it has any.
    fn tool_settings(&self, tool: Tool, cx: &mut Context<Self>) -> Option<AnyElement> {
        match tool {
            Tool::Brush | Tool::Eraser => {
                let (size, hardness, opacity) = self.tool_options.brush(cx);
                let options = &self.tool_options;
                Some(
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
                    ))
                    .into_any_element(),
                )
            }
            Tool::Wand => {
                let tolerance = self.tool_options.tolerance(cx);
                Some(
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
                        )
                        .into_any_element(),
                )
            }
            Tool::Move => Some(
                toggle(
                    "auto-select",
                    "Select layer on click",
                    self.tool_options.auto_select,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.tool_options.auto_select = !this.tool_options.auto_select;
                    cx.notify();
                }))
                .into_any_element(),
            ),
            _ => None,
        }
    }
}
