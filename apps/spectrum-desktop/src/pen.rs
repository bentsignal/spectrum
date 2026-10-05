//! The Pen: click for corner points, press and drag to pull out a curve's
//! handles, and hold Shift to keep a point or handle at 45 degree steps.
//! Enter or a click on the first point closes the shape into a filled path
//! layer, as `spectrum canvas path`; Escape drops it.
use crate::workspace::Workspace;
use gpui::{prelude::*, *};
use spectrum_canvas::{Command, PathAnchor, PathFillRule, PathGeometry};

/// How close, in screen pixels, a click must come to the first point to
/// close the shape.
const CLOSE: f32 = 8.;

/// A placed point and its outgoing handle (the incoming one mirrors it).
pub type PenPoint = ((f32, f32), (f32, f32));

/// `to` moved onto the nearest 45 degree line through `from`.
fn constrained(from: (f32, f32), to: (f32, f32)) -> (f32, f32) {
    let (dx, dy) = (to.0 - from.0, to.1 - from.1);
    let angle = (dy.atan2(dx) / std::f32::consts::FRAC_PI_4).round() * std::f32::consts::FRAC_PI_4;
    let length = dx * angle.cos() + dy * angle.sin();
    (from.0 + length * angle.cos(), from.1 + length * angle.sin())
}

impl Workspace {
    /// A press with the Pen: closes the shape on the first point, or places
    /// a point (along 45 degrees from the last with Shift) whose handle a
    /// drag will pull out.
    pub fn pen_down(
        &mut self,
        at: (f32, f32),
        shift: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (_, scale) = self.canvas_rect();
        let Some(canvas) = &mut self.canvas else {
            return;
        };
        let closes = canvas.pen.len() >= 2
            && canvas
                .pen
                .first()
                .is_some_and(|(f, _)| (f.0 - at.0).hypot(f.1 - at.1) * scale <= CLOSE);
        if closes {
            return self.finish_pen(window, cx);
        }
        let at = match canvas.pen.last() {
            Some((last, _)) if shift => constrained(*last, at),
            _ => at,
        };
        canvas.pen.push((at, (0., 0.)));
        canvas.pen_dragging = true;
        cx.notify();
    }

    /// Dragging after a press pulls out the new point's handles.
    pub fn pen_drag(&mut self, at: (f32, f32), shift: bool, cx: &mut Context<Self>) {
        let Some(canvas) = &mut self.canvas else {
            return;
        };
        if !canvas.pen_dragging {
            return;
        }
        if let Some((point, handle)) = canvas.pen.last_mut() {
            let to = if shift { constrained(*point, at) } else { at };
            *handle = (to.0 - point.0, to.1 - point.1);
        }
        cx.notify();
    }

    pub fn pen_up(&mut self) {
        if let Some(canvas) = &mut self.canvas {
            canvas.pen_dragging = false;
        }
    }

    /// The Pen's points become a filled path layer in the foreground color.
    pub fn finish_pen(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let color = self.colors.fore;
        let Some(canvas) = &mut self.canvas else {
            return;
        };
        canvas.pen_dragging = false;
        let points = std::mem::take(&mut canvas.pen);
        if points.len() < 3 && points.iter().all(|(_, h)| *h == (0., 0.)) {
            return cx.notify();
        }
        if points.len() < 2 {
            return cx.notify();
        }
        // The box must hold every point and handle.
        let reach = points
            .iter()
            .flat_map(|&(p, h)| [p, (p.0 + h.0, p.1 + h.1), (p.0 - h.0, p.1 - h.1)]);
        let (min_x, min_y, max_x, max_y) = reach.fold(
            (f32::MAX, f32::MAX, f32::MIN, f32::MIN),
            |(a, b, c, d), (x, y)| (a.min(x), b.min(y), c.max(x), d.max(y)),
        );
        let (width, height) = (
            (max_x - min_x).ceil().max(1.),
            (max_y - min_y).ceil().max(1.),
        );
        let anchors: Vec<PathAnchor> = points
            .iter()
            .map(|&(p, h)| PathAnchor {
                point: [p.0 - min_x, p.1 - min_y],
                handle_in: [-h.0, -h.1],
                handle_out: [h.0, h.1],
            })
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

    /// The shape so far: its curves, a line on to the pointer, the points,
    /// and the handle being pulled.
    pub fn pen_overlay(&self, offset: Point<Pixels>, scale: f32) -> Option<AnyElement> {
        let canvas = self.canvas.as_ref()?;
        if canvas.tool != crate::tools::Tool::Pen || canvas.pen.is_empty() {
            return None;
        }
        let points = canvas.pen.clone();
        let pointer = canvas.pointer.filter(|_| !canvas.pen_dragging);
        Some(
            gpui::canvas(
                |_, _, _| {},
                move |bounds, _, window, _| {
                    let at = |(x, y): (f32, f32)| {
                        bounds.origin + offset + point(px(x * scale), px(y * scale))
                    };
                    let line = hsla(0.58, 0.8, 0.62, 1.);
                    let mut path = PathBuilder::stroke(px(1.5));
                    path.move_to(at(points[0].0));
                    for pair in points.windows(2) {
                        let ((a, ha), (b, hb)) = (pair[0], pair[1]);
                        path.cubic_bezier_to(
                            at(b),
                            at((a.0 + ha.0, a.1 + ha.1)),
                            at((b.0 - hb.0, b.1 - hb.1)),
                        );
                    }
                    if let (Some(p), Some(&(last, h))) = (pointer, points.last()) {
                        path.cubic_bezier_to(at(p), at((last.0 + h.0, last.1 + h.1)), at(p));
                    }
                    if let Ok(path) = path.build() {
                        window.paint_path(path, line);
                    }
                    for &(p, h) in &points {
                        if h != (0., 0.) {
                            let mut arm = PathBuilder::stroke(px(1.));
                            arm.move_to(at((p.0 - h.0, p.1 - h.1)));
                            arm.line_to(at((p.0 + h.0, p.1 + h.1)));
                            if let Ok(arm) = arm.build() {
                                window.paint_path(arm, line.opacity(0.7));
                            }
                            for end in [(p.0 - h.0, p.1 - h.1), (p.0 + h.0, p.1 + h.1)] {
                                window.paint_quad(
                                    fill(Bounds::centered_at(at(end), size(px(6.), px(6.))), line)
                                        .corner_radii(px(3.)),
                                );
                            }
                        }
                        window.paint_quad(
                            fill(
                                Bounds::centered_at(at(p), size(px(7.), px(7.))),
                                hsla(0., 0., 1., 1.),
                            )
                            .border_widths(px(1.))
                            .border_color(line),
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
