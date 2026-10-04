//! The Brush and Eraser. A stroke shows at once as round dabs along the
//! pointer's path (only inside the selection), while its Paint layer
//! re-renders with the stroke applied, so the canvas shows exactly what the
//! stroke will leave, erasing included. Lifting the pointer sends the same
//! command the preview used, as `spectrum canvas paint`.
use crate::{tools::Tool, workspace::Workspace};
use gpui::{prelude::*, *};
use prism_core::{
    BrushMode, BrushSample, BrushStroke, BrushStyle, Command, LayerKind, PaintSelection,
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
        let (size, ..) = self.tool_options.brush(cx);
        let Some(command) = self.stroke_command(tool, cx) else {
            return;
        };
        let Some(canvas) = &mut self.canvas else {
            return;
        };
        // The layer the stroke lands on: the selected Paint layer, or the
        // one the command adds.
        let target = match &command {
            Command::AddBrushStroke { id, .. } => *id,
            _ => canvas.doc.next_id,
        };
        let samples = match &command {
            Command::AddBrushStroke { stroke, .. }
            | Command::AddPaintLayerWithStroke { stroke, .. } => stroke.samples.clone(),
            _ => return,
        };
        let mut local = prism_core::Workspace::new(canvas.doc.clone(), None);
        if local.execute(command).is_err() {
            return;
        }
        let doc = local.document;
        // Only where the stroke is: its samples' box, widened by the brush.
        let Ok(layer) = doc.layer(target) else {
            return;
        };
        let LayerKind::Paint { program } = &layer.kind else {
            return;
        };
        let reach = size / 2. + 2.;
        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for sample in samples.iter() {
            x0 = x0.min(sample.x - reach);
            y0 = y0.min(sample.y - reach);
            x1 = x1.max(sample.x + reach);
            y1 = y1.max(sample.y + reach);
        }
        let left = x0.floor().clamp(0., program.width as f32) as u32;
        let top = y0.floor().clamp(0., program.height as f32) as u32;
        let right = x1.ceil().clamp(0., program.width as f32) as u32;
        let bottom = y1.ceil().clamp(0., program.height as f32) as u32;
        if right <= left || bottom <= top {
            return;
        }
        let region = (left, top, right - left, bottom - top);
        let Ok(pixels) = prism_core::render_paint_layer_region(&doc, target, region) else {
            return;
        };
        let t = layer.transform;
        let image = crate::preview::to_render_image(image::DynamicImage::ImageRgba8(pixels)).0;
        let bounds = (
            [t.x + left as f32 * t.scale_x, t.y + top as f32 * t.scale_y],
            [
                t.x + right as f32 * t.scale_x,
                t.y + bottom as f32 * t.scale_y,
            ],
        );
        if let Some(old) = canvas
            .stroke_patch
            .replace(crate::canvas_state::StrokePatch {
                layer: target,
                bounds,
                image,
                committed: false,
            })
        {
            window.drop_image(old.image).ok();
        }
        canvas.live_doc = Some(doc);
        // Canvases drawn as one render show the stroke through it instead.
        if !crate::layer_cache::stackable(&canvas.doc) {
            canvas.bump_version();
            self.render_canvas(window, cx);
        }
    }

    /// Ends a stroke: the same command the preview showed is applied and saved.
    pub fn finish_stroke(&mut self, tool: Tool, window: &mut Window, cx: &mut Context<Self>) {
        let command = self.stroke_command(tool, cx);
        if let Some(canvas) = &mut self.canvas {
            canvas.points.clear();
            canvas.live_doc = None;
            // The patch stays until the layer's new render arrives.
            if let Some(patch) = &mut canvas.stroke_patch {
                patch.committed = true;
            }
        }
        match command {
            Some(command) => self.canvas_commands(vec![command], window, cx),
            None => cx.notify(),
        }
    }

    /// The brush's outline and a small crosshair at the pointer. The system
    /// cursor is hidden over the canvas with a brush, so these move together.
    pub fn brush_overlay(&self, offset: Point<Pixels>, scale: f32, cx: &App) -> Option<AnyElement> {
        let canvas = self.canvas.as_ref()?;
        if !matches!(canvas.tool, Tool::Brush | Tool::Eraser) {
            return None;
        }
        let (brush, ..) = self.tool_options.brush(cx);
        let pointer = canvas.pointer?;
        Some(
            gpui::canvas(
                |_, _, _| {},
                move |bounds, _, window, _| {
                    let center = bounds.origin
                        + offset
                        + point(px(pointer.0 * scale), px(pointer.1 * scale));
                    let radius = (brush * scale / 2.).max(1.);
                    let ring = Bounds::centered_at(center, size(px(radius * 2.), px(radius * 2.)));
                    // A dark ring inside a light one reads on any color.
                    window.paint_quad(
                        outline(ring, hsla(0., 0., 1., 0.85), BorderStyle::Solid)
                            .corner_radii(px(radius)),
                    );
                    let inner = Bounds::centered_at(
                        center,
                        size(
                            px((radius - 1.).max(0.5) * 2.),
                            px((radius - 1.).max(0.5) * 2.),
                        ),
                    );
                    window.paint_quad(
                        outline(inner, hsla(0., 0., 0., 0.5), BorderStyle::Solid)
                            .corner_radii(px(radius - 1.)),
                    );
                    for (dx, dy) in [(1., 0.), (0., 1.)] {
                        let mut tick = PathBuilder::stroke(px(1.));
                        tick.move_to(center - point(px(4. * dx), px(4. * dy)));
                        tick.line_to(center + point(px(4. * dx), px(4. * dy)));
                        if let Ok(path) = tick.build() {
                            window.paint_path(path, hsla(0., 0., 1., 0.9));
                        }
                    }
                },
            )
            .absolute()
            .size_full()
            .into_any_element(),
        )
    }
}
