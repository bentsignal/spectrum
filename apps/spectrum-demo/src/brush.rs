//! The Brush and Eraser. A stroke shows at once as round dabs along the
//! pointer's path (only inside the selection), while its Paint layer
//! re-renders with the stroke applied, so the canvas shows exactly what the
//! stroke will leave, erasing included. Lifting the pointer sends the same
//! command the preview used, as `spectrum canvas paint`.
use crate::{tools::Tool, workspace::Workspace};
use gpui::{prelude::*, *};
use prism_core::{
    BrushMode, BrushSample, BrushStroke, BrushStyle, Command, LayerKind, PaintSelection, Selection,
};

impl Workspace {
    /// The selected Paint layer, if one is selected.
    fn paint_layer(&self) -> Option<(u64, prism_core::Transform)> {
        let canvas = self.canvas.as_ref()?;
        let layer = canvas.doc.layer(canvas.selected?).ok()?;
        matches!(layer.kind, LayerKind::Paint { .. }).then_some((layer.id, layer.transform))
    }

    /// Whether a stroke can start; the Eraser needs a Paint layer to erase.
    pub fn can_stroke(&mut self, tool: Tool, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if tool == Tool::Eraser && self.paint_layer().is_none() {
            self.notify_error(
                anyhow::anyhow!("Select a Paint layer to erase. The Brush paints on a new one."),
                window,
                cx,
            );
            return false;
        }
        true
    }

    /// The command the stroke drawn so far makes: onto the selected Paint
    /// layer, or a new Paint layer covering the canvas.
    fn stroke_command(&self, tool: Tool, cx: &App) -> Option<Command> {
        let canvas = self.canvas.as_ref()?;
        let (size, hardness, opacity) = self.tool_options.brush(cx);
        let (width, height) = (canvas.doc.width, canvas.doc.height);
        let paint = self.paint_layer();
        if tool == Tool::Eraser && paint.is_none() {
            return None;
        }
        // Samples in the Paint layer's own pixels.
        let local = |(x, y): (f32, f32)| match paint {
            Some((_, t)) => (
                ((x - t.x) / t.scale_x.max(1e-3)).clamp(0., width as f32),
                ((y - t.y) / t.scale_y.max(1e-3)).clamp(0., height as f32),
            ),
            None => (x.clamp(0., width as f32), y.clamp(0., height as f32)),
        };
        let mut samples: Vec<BrushSample> = canvas
            .points
            .iter()
            .map(|&p| {
                let (x, y) = local(p);
                BrushSample { x, y, pressure: 1. }
            })
            .collect();
        if samples.is_empty() {
            return None;
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
            color: self.colors.fore,
            size,
            hardness,
            opacity,
            ..BrushStyle::default()
        };
        let stroke = BrushStroke::new(style, samples).ok()?;
        Some(match paint {
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
        })
    }

    /// Applies the stroke so far to a copy of the document, so its layer
    /// renders with it while the pointer is still down.
    pub fn update_live_stroke(&mut self, tool: Tool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(command) = self.stroke_command(tool, cx) else {
            return;
        };
        let Some(canvas) = &mut self.canvas else {
            return;
        };
        let mut local = prism_core::Workspace::new(canvas.doc.clone(), None);
        if local.execute(command).is_ok() {
            canvas.live_doc = Some(local.document);
            canvas.bump_version();
            self.render_canvas(window, cx);
            self.refresh_layers(window, cx);
        }
    }

    /// Ends a stroke: the same command the preview showed is applied and saved.
    pub fn finish_stroke(&mut self, tool: Tool, window: &mut Window, cx: &mut Context<Self>) {
        let command = self.stroke_command(tool, cx);
        if let Some(canvas) = &mut self.canvas {
            canvas.points.clear();
            canvas.live_doc = None;
        }
        match command {
            Some(command) => self.canvas_commands(vec![command], window, cx),
            None => cx.notify(),
        }
    }

    /// The stroke being drawn as round dabs inside the selection, and the
    /// brush's outline at the pointer.
    pub fn brush_overlay(&self, offset: Point<Pixels>, scale: f32, cx: &App) -> Option<AnyElement> {
        let canvas = self.canvas.as_ref()?;
        if !matches!(canvas.tool, Tool::Brush | Tool::Eraser) {
            return None;
        }
        let (brush, _, opacity) = self.tool_options.brush(cx);
        let eraser = canvas.tool == Tool::Eraser;
        let points = canvas.points.clone();
        let pointer = canvas.pointer;
        let selection = canvas.doc.selection.clone();
        let [r, g, b, a] = self.colors.fore;
        let color = Hsla::from(rgba(u32::from_be_bytes([r, g, b, 255])))
            .opacity(opacity * f32::from(a) / 255.);
        let inside = move |(x, y): (f32, f32)| selection.as_ref().is_none_or(|s| selected(s, x, y));
        Some(
            gpui::canvas(
                |_, _, _| {},
                move |bounds, _, window, _| {
                    let at = |(x, y): (f32, f32)| {
                        bounds.origin + offset + point(px(x * scale), px(y * scale))
                    };
                    let radius = (brush * scale / 2.).max(0.5);
                    if !eraser && !points.is_empty() {
                        // Dabs a quarter of the brush apart along the path.
                        let step = (brush / 4.).max(0.5);
                        let mut dab = |p: (f32, f32)| {
                            if inside(p) {
                                let quad = fill(
                                    Bounds::centered_at(
                                        at(p),
                                        size(px(radius * 2.), px(radius * 2.)),
                                    ),
                                    color,
                                )
                                .corner_radii(px(radius));
                                window.paint_quad(quad);
                            }
                        };
                        dab(points[0]);
                        for pair in points.windows(2) {
                            let (a, b) = (pair[0], pair[1]);
                            let length = (b.0 - a.0).hypot(b.1 - a.1);
                            let count = (length / step).ceil() as usize;
                            for i in 1..=count {
                                let t = i as f32 / count as f32;
                                dab((a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t));
                            }
                        }
                    }
                    if let Some(p) = pointer {
                        let ring =
                            Bounds::centered_at(at(p), size(px(radius * 2.), px(radius * 2.)));
                        window.paint_quad(
                            outline(ring, hsla(0., 0., 1., 0.8), BorderStyle::Solid)
                                .corner_radii(px(radius)),
                        );
                    }
                },
            )
            .absolute()
            .size_full()
            .into_any_element(),
        )
    }
}

/// Whether the canvas point is selected (at least half).
fn selected(selection: &Selection, x: f32, y: f32) -> bool {
    let (sx, sy, w, h) = selection.bounds();
    let (px, py) = (
        x.floor() as i64 - i64::from(sx),
        y.floor() as i64 - i64::from(sy),
    );
    if px < 0 || py < 0 || px >= i64::from(w) || py >= i64::from(h) {
        return false;
    }
    selection
        .alpha()
        .is_none_or(|alpha| alpha[(py as u32 * w + px as u32) as usize] >= 128)
}
