//! Crop: drag the area to keep; what falls outside is shaded while the drag
//! goes on, and release crops the canvas, as `spectrum canvas crop`.
use crate::{tools::Tool, workspace::Workspace};
use gpui::{prelude::*, *};
use prism_core::Command;

impl Workspace {
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

    /// The crop being dragged: the box to keep, with the rest shaded.
    pub fn crop_overlay(&self, offset: Point<Pixels>, scale: f32) -> Option<AnyElement> {
        let canvas = self.canvas.as_ref()?;
        let crop = (canvas.tool == Tool::Crop)
            .then_some(canvas.creating)
            .flatten();
        crop?;
        let at = move |origin: Point<Pixels>, (x, y): (f32, f32)| {
            origin + offset + point(px(x * scale), px(y * scale))
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
}
