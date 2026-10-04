//! The tool options bar above the canvas, as in Photoshop: the current
//! tool, its settings (brush size, wand tolerance), what a selection can
//! do, and a line on how to use the tool. It keeps one height, so nothing
//! below it moves as tools change.
use crate::{
    controls::{inline_slider, toggle},
    theme::*,
    tools::{Tool, tool_glyph, tool_name},
    workspace::Workspace,
};
use gpui::{prelude::*, *};

pub const BAR_HEIGHT: f32 = 40.;

/// A small bordered button for the bar.
fn bar_button(id: &'static str, label: &'static str) -> Stateful<Div> {
    div()
        .id(id)
        .h(px(24.))
        .px_2()
        .flex()
        .items_center()
        .rounded_md()
        .border_1()
        .border_color(rgb(BORDER))
        .text_xs()
        .text_color(rgb(MUTED))
        .cursor_pointer()
        .hover(|el| el.text_color(rgb(TEXT)).bg(rgb(HOVER)))
        .child(label)
}

fn hint(tool: Tool) -> &'static str {
    match tool {
        Tool::Move => "Drag to move · corners resize · the top handle rotates (Shift: 15° steps)",
        Tool::Marquee | Tool::EllipseSelect | Tool::Lasso | Tool::Wand => {
            "Shift adds · Option subtracts · both intersect · click off the canvas to deselect"
        }
        Tool::Brush => "Paints the foreground color; a selection limits where paint goes",
        Tool::Eraser => "Erases paint on the selected Paint layer",
        Tool::Text => "Click to place text",
        Tool::Box | Tool::Circle => "Drag to draw in the foreground color · Shift+U switches",
        Tool::Gradient => "Drag from where the gradient starts to where it ends",
        Tool::Pen => {
            "Click for corners · drag for curves · Shift keeps 45° · Enter or the first point closes"
        }
        Tool::Crop => "Drag the area to keep",
        Tool::Eyedropper => "Click to take a color · Option-click sets the background",
        Tool::Image => "Place an image from the project",
    }
}

impl Workspace {
    pub fn options_bar(&self, cx: &mut Context<Self>) -> Option<Div> {
        let canvas = self.canvas.as_ref()?;
        let tool = canvas.tool;
        let selected = canvas.doc.selection.is_some();
        let mut settings = div().flex().items_center().gap_5();
        match tool {
            Tool::Brush | Tool::Eraser => {
                let (size, hardness, opacity) = self.tool_options.brush(cx);
                let options = &self.tool_options;
                settings = settings
                    .child(inline_slider(
                        "Size",
                        format!("{size:.0}"),
                        &options.brush_size,
                        110.,
                    ))
                    .child(inline_slider(
                        "Hardness",
                        format!("{:.0}%", hardness * 100.),
                        &options.brush_hardness,
                        90.,
                    ))
                    .child(inline_slider(
                        "Opacity",
                        format!("{:.0}%", opacity * 100.),
                        &options.brush_opacity,
                        90.,
                    ));
            }
            Tool::Wand => {
                let tolerance = self.tool_options.tolerance(cx);
                settings = settings
                    .child(inline_slider(
                        "Tolerance",
                        tolerance.to_string(),
                        &self.tool_options.wand_tolerance,
                        110.,
                    ))
                    .child(
                        toggle(
                            "wand-contiguous",
                            "Contiguous",
                            self.tool_options.contiguous,
                        )
                        .text_xs()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.tool_options.contiguous = !this.tool_options.contiguous;
                            cx.notify();
                        })),
                    );
            }
            _ => {}
        }
        let has_settings = matches!(tool, Tool::Brush | Tool::Eraser | Tool::Wand);
        let action = |id: &'static str,
                      label: &'static str,
                      run: fn(&mut Self, &mut Window, &mut Context<Self>)| {
            bar_button(id, label)
                .on_click(cx.listener(move |this, _, window, cx| run(this, window, cx)))
        };
        let selection = (selected || tool.selects()).then(|| {
            div()
                .flex()
                .items_center()
                .gap_1p5()
                .child(action("select-all", "All", Self::select_canvas))
                .when(selected, |el| {
                    el.child(action("select-none", "Deselect", Self::deselect))
                        .child(action("select-invert", "Invert", Self::invert_selection))
                        .child(div().w(px(8.)))
                        .child(action("selection-fill", "Fill", Self::fill_selection))
                        .child(action(
                            "selection-mask",
                            "Mask layer",
                            Self::mask_to_selection,
                        ))
                        .child(action(
                            "selection-delete",
                            "Hide",
                            Self::delete_in_selection,
                        ))
                        .child(action("selection-crop", "Crop", Self::crop_to_selection))
                })
        });
        Some(
            div()
                .h(px(BAR_HEIGHT))
                .flex_shrink_0()
                .px_6()
                .flex()
                .items_center()
                .gap_5()
                .border_b_1()
                .border_color(rgb(BORDER))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_sm()
                        .child(tool_glyph(tool))
                        .child(tool_name(tool)),
                )
                .when(has_settings, |el| el.child(settings))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_xs()
                        .text_color(rgb(FAINT))
                        // Settings take the room; the hint is for tools without.
                        .when(!has_settings, |el| el.child(hint(tool))),
                )
                .children(selection),
        )
    }
}
