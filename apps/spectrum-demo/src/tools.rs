//! Canvas tools. One tool is current; its button and Command+P open a list
//! of tools. Text places text where you click; Box and Circle draw by
//! dragging (a click makes a default size). After placing, Move returns.
use crate::{theme::*, workspace::Workspace};
use gpui::{prelude::*, *};
use gpui_component::{Icon, IconName, Sizable};
use prism_core::{Command, GradientStop, ShapeFill, ShapeGradient};

#[derive(Clone, Copy, PartialEq, Default)]
pub enum Tool {
    #[default]
    Move,
    Text,
    Box,
    Circle,
    Gradient,
    Image,
    Marquee,
    EllipseSelect,
    Lasso,
    Wand,
    Brush,
    Eraser,
    Crop,
    Pen,
}

impl Tool {
    /// Tools that make or change the selection; they stay current after use.
    pub fn selects(self) -> bool {
        matches!(
            self,
            Tool::Marquee | Tool::EllipseSelect | Tool::Lasso | Tool::Wand
        )
    }
}

pub const TOOLS: [(Tool, &str, &str); 14] = [
    (Tool::Move, "Move", "Select, move, and resize layers"),
    (
        Tool::Marquee,
        "Marquee",
        "Drag to select a box; Shift adds, Option subtracts",
    ),
    (
        Tool::EllipseSelect,
        "Ellipse select",
        "Drag to select an ellipse; Shift adds, Option subtracts",
    ),
    (
        Tool::Lasso,
        "Lasso",
        "Draw around what to select; Shift adds, Option subtracts",
    ),
    (Tool::Wand, "Magic wand", "Click to select similar colors"),
    (
        Tool::Brush,
        "Brush",
        "Paint with the foreground color on a Paint layer",
    ),
    (Tool::Eraser, "Eraser", "Erase paint on a Paint layer"),
    (Tool::Text, "Text", "Click to place text"),
    (Tool::Box, "Box", "Drag to draw a box"),
    (Tool::Circle, "Circle", "Drag to draw a circle"),
    (
        Tool::Gradient,
        "Gradient",
        "Drag to fill a new layer from the foreground to the background color",
    ),
    (
        Tool::Pen,
        "Pen",
        "Click points to draw a shape; click the first point or press Enter to close",
    ),
    (Tool::Crop, "Crop", "Drag the area to crop the canvas to"),
    (Tool::Image, "Image", "Place an image from the project"),
];

const SHORTCUT: &str = if cfg!(target_os = "macos") {
    "⌘P"
} else {
    "Ctrl+P"
};

pub fn tool_name(tool: Tool) -> &'static str {
    TOOLS
        .iter()
        .find(|(t, ..)| *t == tool)
        .map_or("Move", |(_, name, ..)| name)
}

fn glyph_text(mark: &'static str) -> AnyElement {
    div()
        .text_sm()
        .text_color(rgb(MUTED))
        .child(mark)
        .into_any_element()
}

/// A small mark for each tool, in the muted text color.
pub fn tool_glyph(tool: Tool) -> AnyElement {
    let outline = || div().size(px(11.)).border_1().border_color(rgb(MUTED));
    let mark = match tool {
        Tool::Box => outline().rounded_xs().into_any_element(),
        Tool::Circle => outline().rounded_full().into_any_element(),
        Tool::Gradient => outline()
            .rounded_xs()
            .bg(linear_gradient(
                90.,
                linear_color_stop(rgb(0x2a2a2a), 0.),
                linear_color_stop(rgb(MUTED), 1.),
            ))
            .into_any_element(),
        Tool::Text => div()
            .text_sm()
            .font_weight(FontWeight::MEDIUM)
            .text_color(rgb(MUTED))
            .child("T")
            .into_any_element(),
        Tool::Marquee => outline().border_dashed().into_any_element(),
        Tool::EllipseSelect => outline().rounded_full().border_dashed().into_any_element(),
        Tool::Lasso => glyph_text("ʘ"),
        Tool::Wand => glyph_text("✦"),
        Tool::Brush => glyph_text("✎"),
        Tool::Eraser => outline().rounded_xs().bg(rgb(MUTED)).into_any_element(),
        Tool::Crop => Icon::new(IconName::Maximize)
            .small()
            .text_color(rgb(MUTED))
            .into_any_element(),
        Tool::Pen => glyph_text("✒"),
        Tool::Image => Icon::new(IconName::Frame)
            .small()
            .text_color(rgb(MUTED))
            .into_any_element(),
        Tool::Move => canvas(
            |_, _, _| {},
            |bounds, _, window, _| {
                // A pointer arrow.
                let o = bounds.origin + point(px(3.), px(1.));
                let mut path = PathBuilder::fill();
                path.move_to(o);
                for (x, y) in [
                    (0., 11.),
                    (3., 8.5),
                    (5.5, 13.),
                    (7., 12.3),
                    (4.7, 7.8),
                    (8.5, 7.8),
                ] {
                    path.line_to(o + point(px(x), px(y)));
                }
                path.close();
                if let Ok(path) = path.build() {
                    window.paint_path(path, rgb(MUTED));
                }
            },
        )
        .size(px(14.))
        .into_any_element(),
    };
    div()
        .size(px(16.))
        .flex()
        .items_center()
        .justify_center()
        .child(mark)
        .into_any_element()
}

impl Workspace {
    pub fn set_tool(&mut self, tool: Tool, window: &mut Window, cx: &mut Context<Self>) {
        if tool == Tool::Image {
            return self.open_place_picker(window, cx);
        }
        if let Some(canvas) = &mut self.canvas {
            canvas.tool = tool;
        }
        self.stop_editing_text(window, cx);
        cx.notify();
    }

    /// Adds a layer for `tool` over canvas-space `start` to `end`; a click
    /// without a drag gets a default size there.
    pub fn create_with_tool(
        &mut self,
        tool: Tool,
        start: (f32, f32),
        end: (f32, f32),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match tool {
            Tool::Gradient => return self.create_gradient(start, end, window, cx),
            Tool::Brush | Tool::Eraser => return self.finish_stroke(tool, window, cx),
            Tool::Crop => return self.crop_canvas(start, end, window, cx),
            tool if tool.selects() => return self.finish_selection(tool, start, end, window, cx),
            Tool::Text | Tool::Box | Tool::Circle => {}
            _ => return,
        }
        let Some(canvas) = &mut self.canvas else {
            return;
        };
        canvas.tool = Tool::Move;
        let short = canvas.doc.width.min(canvas.doc.height) as f32;
        let side = (short * 0.4).round().max(8.);
        let (mut min, mut max) = (
            (start.0.min(end.0), start.1.min(end.1)),
            (start.0.max(end.0), start.1.max(end.1)),
        );
        if max.0 - min.0 < 4. && max.1 - min.1 < 4. {
            let (w, h) = if tool == Tool::Box {
                (side * 1.4, side)
            } else {
                (side, side)
            };
            min = (start.0 - w / 2., start.1 - h / 2.);
            max = (start.0 + w / 2., start.1 + h / 2.);
        }
        let (width, height) = (
            (max.0 - min.0).round().max(1.) as u32,
            (max.1 - min.1).round().max(1.) as u32,
        );
        let color = self.colors.fore;
        let command = match tool {
            Tool::Text => Command::AddText {
                text: "Text".into(),
                name: None,
                font_size: (short * 0.12).round().max(12.),
                color,
                x: start.0,
                y: start.1,
                shaping: Default::default(),
            },
            Tool::Circle => Command::AddEllipse {
                name: None,
                width,
                height,
                color,
                x: min.0,
                y: min.1,
            },
            _ => Command::AddRectangle {
                name: None,
                width,
                height,
                color,
                corner_radius: 0.,
                x: min.0,
                y: min.1,
            },
        };
        self.canvas_commands(vec![command], window, cx);
        if tool == Tool::Text {
            self.edit_text(window, cx);
            // Start empty so typing replaces the placeholder text; empty text
            // is never sent, so the layer keeps "Text" until something is typed.
            self.text_input
                .update(cx, |state, cx| state.set_value("", window, cx));
        }
    }

    /// A new layer covering the canvas, filled from the foreground to the
    /// background color along the drag from `start` to `end`.
    fn create_gradient(
        &mut self,
        start: (f32, f32),
        end: (f32, f32),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(canvas) = &mut self.canvas else {
            return;
        };
        canvas.tool = Tool::Move;
        let (width, height) = (canvas.doc.width, canvas.doc.height);
        let id = canvas.doc.next_id;
        let (w, h) = (width.max(1) as f32, height.max(1) as f32);
        // Linear gradients run in the layer's normalized box: the drag's
        // direction there, and where it starts and how far it reaches.
        let a = (start.0 / w, start.1 / h);
        let d = (end.0 / w - a.0, end.1 / h - a.1);
        let length = d.0.hypot(d.1);
        let (angle, offset, extent) = if length < 0.01 {
            (0., 0., 1.)
        } else {
            let dir = (d.0 / length, d.1 / length);
            let angle = dir.1.atan2(dir.0).to_degrees().rem_euclid(360.);
            let offset = (a.0 - 0.5) * dir.0 + (a.1 - 0.5) * dir.1 + 0.5;
            (angle, offset, length)
        };
        let gradient = ShapeGradient {
            angle,
            offset,
            extent,
            stops: vec![
                GradientStop::new(0., self.colors.fore),
                GradientStop::new(1., self.colors.back),
            ],
            ..ShapeGradient::default()
        };
        self.canvas_commands(
            vec![
                Command::AddRectangle {
                    name: Some("Gradient".into()),
                    width,
                    height,
                    color: self.colors.fore,
                    corner_radius: 0.,
                    x: 0.,
                    y: 0.,
                },
                Command::SetShapeFill {
                    id,
                    fill: Some(ShapeFill::Gradient(gradient)),
                },
            ],
            window,
            cx,
        );
    }

    /// The current tool, as a button that opens the tool list.
    pub fn tool_button(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let tool = self.canvas.as_ref().map_or(Tool::Move, |c| c.tool);
        div()
            .id("tool")
            .h(px(34.))
            .px_3()
            .flex()
            .items_center()
            .gap_2p5()
            .rounded_md()
            .border_1()
            .border_color(rgb(BORDER))
            .bg(rgb(SURFACE))
            .hover(|el| el.bg(rgb(HOVER)))
            .child(tool_glyph(tool))
            .child(div().flex_1().text_sm().child(tool_name(tool)))
            .child(div().text_xs().text_color(rgb(FAINT)).child(SHORTCUT))
            .on_click(cx.listener(|this, _, window, cx| this.open_tools(window, cx)))
    }
}
