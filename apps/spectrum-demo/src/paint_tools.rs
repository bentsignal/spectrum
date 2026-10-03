//! Brush and Eraser strokes on Paint layers, the Pen, and Crop. A stroke
//! shows on screen as it is drawn and becomes one engine command when the
//! pointer lifts, as `spectrum canvas paint`; the Pen adds a path layer
//! (`spectrum canvas path`) and Crop crops the canvas (`spectrum canvas crop`).
use crate::{
    controls::{group, slider_row},
    theme::*,
    tools::Tool,
    workspace::Workspace,
};
use gpui::{prelude::*, *};
use prism_core::{
    BrushMode, BrushSample, BrushStroke, BrushStyle, Command, LayerKind, PaintSelection,
    PathAnchor, PathFillRule, PathGeometry,
};

/// How close, in screen pixels, a Pen click must come to the first point to
/// close the shape.
const CLOSE: f32 = 8.;

impl Workspace {
    /// Ends a Brush or Eraser stroke: the selected Paint layer takes it, or
    /// a new Paint layer covering the canvas does.
    pub fn finish_stroke(&mut self, tool: Tool, window: &mut Window, cx: &mut Context<Self>) {
        let (size, hardness, opacity) = self.tool_options.brush(cx);
        let color = self.colors.fore;
        let Some(canvas) = &mut self.canvas else {
            return;
        };
        let points = std::mem::take(&mut canvas.points);
        let (width, height) = (canvas.doc.width, canvas.doc.height);
        let paint = canvas
            .selected
            .and_then(|id| canvas.doc.layer(id).ok())
            .filter(|l| matches!(l.kind, LayerKind::Paint { .. }))
            .map(|l| (l.id, l.transform));
        if tool == Tool::Eraser && paint.is_none() {
            return self.notify_error(
                anyhow::anyhow!("Select a Paint layer to erase; the Brush makes one."),
                window,
                cx,
            );
        }
        // Samples in the Paint layer's own pixels.
        let local = |(x, y): (f32, f32)| match paint {
            Some((_, t)) => (
                ((x - t.x) / t.scale_x.max(1e-3)).clamp(0., width as f32),
                ((y - t.y) / t.scale_y.max(1e-3)).clamp(0., height as f32),
            ),
            None => (x.clamp(0., width as f32), y.clamp(0., height as f32)),
        };
        let mut samples: Vec<BrushSample> = points
            .iter()
            .map(|&p| {
                let (x, y) = local(p);
                BrushSample { x, y, pressure: 1. }
            })
            .collect();
        if samples.is_empty() {
            return;
        }
        if samples.len() == 1 {
            samples.push(samples[0]);
        }
        let style = BrushStyle {
            mode: if tool == Tool::Eraser {
                BrushMode::Erase
            } else {
                BrushMode::Paint
            },
            color,
            size,
            hardness,
            opacity,
            ..BrushStyle::default()
        };
        let stroke = match BrushStroke::new(style, samples) {
            Ok(stroke) => stroke,
            Err(error) => return self.notify_error(error, window, cx),
        };
        let command = match paint {
            Some((id, _)) => Command::AddBrushStroke {
                id,
                stroke,
                selection: PaintSelection::Current,
            },
            None => Command::AddPaintLayerWithStroke {
                name: Some("Paint".into()),
                width,
                height,
                stroke,
                selection: PaintSelection::Current,
            },
        };
        self.canvas_commands(vec![command], window, cx);
    }

    /// A Pen click: adds an anchor, or closes the shape on the first one.
    pub fn pen_click(&mut self, at: (f32, f32), window: &mut Window, cx: &mut Context<Self>) {
        let (_, scale) = self.canvas_rect();
        let Some(canvas) = &mut self.canvas else {
            return;
        };
        let closes = canvas.pen.len() >= 3
            && canvas
                .pen
                .first()
                .is_some_and(|f| (f.0 - at.0).hypot(f.1 - at.1) * scale <= CLOSE);
        if closes {
            return self.finish_pen(window, cx);
        }
        canvas.pen.push(at);
        cx.notify();
    }

    /// Enter, or a click on the first point: the Pen's points become a
    /// filled path layer in the foreground color.
    pub fn finish_pen(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let color = self.colors.fore;
        let Some(canvas) = &mut self.canvas else {
            return;
        };
        let points = std::mem::take(&mut canvas.pen);
        if points.len() < 3 {
            return cx.notify();
        }
        let (min_x, min_y) = points
            .iter()
            .fold((f32::MAX, f32::MAX), |(x, y), p| (x.min(p.0), y.min(p.1)));
        let (max_x, max_y) = points
            .iter()
            .fold((f32::MIN, f32::MIN), |(x, y), p| (x.max(p.0), y.max(p.1)));
        let (width, height) = (
            (max_x - min_x).ceil().max(1.),
            (max_y - min_y).ceil().max(1.),
        );
        let anchors: Vec<PathAnchor> = points
            .iter()
            .map(|p| PathAnchor::corner(p.0 - min_x, p.1 - min_y))
            .collect();
        let geometry = PathGeometry::new(
            width as u32,
            height as u32,
            true,
            PathFillRule::EvenOdd,
            anchors,
        );
        match geometry {
            Ok(geometry) => self.canvas_commands(
                vec![Command::AddPath {
                    name: Some("Shape".into()),
                    geometry,
                    color,
                    x: min_x,
                    y: min_y,
                }],
                window,
                cx,
            ),
            Err(error) => self.notify_error(error, window, cx),
        }
    }

    /// Crops the canvas to the dragged box.
    pub fn crop_canvas(
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
        let (w, h) = (canvas.doc.width as f32, canvas.doc.height as f32);
        let x0 = start.0.min(end.0).clamp(0., w).round();
        let y0 = start.1.min(end.1).clamp(0., h).round();
        let x1 = start.0.max(end.0).clamp(0., w).round();
        let y1 = start.1.max(end.1).clamp(0., h).round();
        if x1 - x0 < 2. || y1 - y0 < 2. {
            return cx.notify();
        }
        self.canvas_commands(
            vec![Command::CropCanvas {
                x: x0 as u32,
                y: y0 as u32,
                width: (x1 - x0) as u32,
                height: (y1 - y0) as u32,
            }],
            window,
            cx,
        );
    }

    /// The stroke, Pen path, or crop being drawn.
    pub fn paint_overlay(&self, offset: Point<Pixels>, scale: f32, cx: &App) -> Option<AnyElement> {
        let canvas = self.canvas.as_ref()?;
        let (brush, _, opacity) = self.tool_options.brush(cx);
        let [r, g, b, _] = self.colors.fore;
        let stroke = matches!(canvas.tool, Tool::Brush | Tool::Eraser)
            .then(|| canvas.points.clone())
            .filter(|p| !p.is_empty());
        let pen = (canvas.tool == Tool::Pen && !canvas.pen.is_empty())
            .then(|| (canvas.pen.clone(), canvas.pointer));
        let crop = (canvas.tool == Tool::Crop)
            .then_some(canvas.creating)
            .flatten();
        let eraser = canvas.tool == Tool::Eraser;
        if stroke.is_none() && pen.is_none() && crop.is_none() {
            return None;
        }
        let at = move |origin: Point<Pixels>, (x, y): (f32, f32)| {
            origin + offset + point(px(x * scale), px(y * scale))
        };
        let color = if eraser {
            hsla(0., 0., 1., 0.35)
        } else {
            Hsla::from(rgb(u32::from_be_bytes([0, r, g, b]))).opacity(opacity)
        };
        let canvas_size = size(
            px(canvas.doc.width as f32 * scale),
            px(canvas.doc.height as f32 * scale),
        );
        Some(
            gpui::canvas(
                |_, _, _| {},
                move |bounds, _, window, _| {
                    let origin = bounds.origin;
                    if let Some(points) = &stroke {
                        let mut path = PathBuilder::stroke(px((brush * scale).max(1.)));
                        path.move_to(at(origin, points[0]));
                        for &p in points.iter().skip(1) {
                            path.line_to(at(origin, p));
                        }
                        if points.len() == 1 {
                            path.line_to(at(origin, points[0]) + point(px(0.5), px(0.)));
                        }
                        if let Ok(path) = path.build() {
                            window.paint_path(path, color);
                        }
                    }
                    if let Some((points, pointer)) = &pen {
                        let mut path = PathBuilder::stroke(px(1.5));
                        path.move_to(at(origin, points[0]));
                        for &p in points.iter().skip(1).chain(pointer.iter()) {
                            path.line_to(at(origin, p));
                        }
                        if let Ok(path) = path.build() {
                            window.paint_path(path, hsla(0.58, 0.8, 0.6, 1.));
                        }
                        for &p in points {
                            let c = at(origin, p);
                            window.paint_quad(fill(
                                Bounds::centered_at(c, size(px(7.), px(7.))),
                                hsla(0., 0., 1., 1.),
                            ));
                        }
                    }
                    if let Some((a, b)) = crop {
                        // Shade what the crop leaves out.
                        let (a, b) = (at(origin, a), at(origin, b));
                        let keep = Bounds::from_corners(
                            point(a.x.min(b.x), a.y.min(b.y)),
                            point(a.x.max(b.x), a.y.max(b.y)),
                        );
                        let whole = Bounds::new(origin + offset, canvas_size);
                        let shade = hsla(0., 0., 0., 0.55);
                        for part in [
                            Bounds::from_corners(whole.origin, point(whole.right(), keep.top())),
                            Bounds::from_corners(
                                point(whole.left(), keep.bottom()),
                                whole.bottom_right(),
                            ),
                            Bounds::from_corners(
                                point(whole.left(), keep.top()),
                                point(keep.left(), keep.bottom()),
                            ),
                            Bounds::from_corners(
                                point(keep.right(), keep.top()),
                                point(whole.right(), keep.bottom()),
                            ),
                        ] {
                            if part.size.width > px(0.) && part.size.height > px(0.) {
                                window.paint_quad(fill(part, shade));
                            }
                        }
                        window.paint_quad(outline(keep, hsla(0., 0., 1., 0.9), BorderStyle::Solid));
                    }
                },
            )
            .absolute()
            .size_full()
            .into_any_element(),
        )
    }

    /// The Tool group's Brush and Eraser settings.
    pub fn brush_panel(&self, cx: &mut Context<Self>) -> Option<Div> {
        let tool = self.canvas.as_ref()?.tool;
        if !matches!(tool, Tool::Brush | Tool::Eraser) {
            return None;
        }
        let (size, hardness, opacity) = self.tool_options.brush(cx);
        Some(
            group(if tool == Tool::Brush { "Brush" } else { "Eraser" }, None)
                .gap_3()
                .child(slider_row("Size", format!("{size:.0}"), &self.tool_options.brush_size))
                .child(slider_row(
                    "Hardness",
                    format!("{:.0}%", hardness * 100.),
                    &self.tool_options.brush_hardness,
                ))
                .child(slider_row(
                    "Opacity",
                    format!("{:.0}%", opacity * 100.),
                    &self.tool_options.brush_opacity,
                ))
                .child(div().text_xs().text_color(rgb(FAINT)).child(
                    if tool == Tool::Brush {
                        "Paints the foreground color on the selected Paint layer, or a new one. A selection limits where paint goes."
                    } else {
                        "Erases paint on the selected Paint layer."
                    },
                )),
        )
    }
}
