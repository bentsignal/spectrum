//! Guides and snapping on the canvas. Guides are document lines; drag one to
//! move it or off the canvas to remove it. With snapping on, a dragged
//! layer's edges and center snap to guides, the canvas, and other layers.
use crate::{canvas_view::whole_pixels, theme::*, workspace::Workspace};
use gpui::{prelude::*, *};
use spectrum_canvas::{Command, GuideOrientation, Snapped};

/// How close, in screen pixels, a pointer or edge must come to a line.
const REACH: f32 = 6.;
const GUIDE: u32 = 0x3fb8d0;
const SMART: u32 = 0xe0508a;

/// A guide being dragged, and where it is now in canvas units.
#[derive(Clone, Copy)]
pub struct GuideDrag {
    pub id: u64,
    pub orientation: GuideOrientation,
    pub position: f32,
}

impl Workspace {
    pub fn toggle_guides(&mut self, cx: &mut Context<Self>) {
        self.guides_visible = !self.guides_visible;
        cx.notify();
    }

    pub fn toggle_snapping(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(canvas) = &self.canvas else {
            return;
        };
        let enabled = !canvas.doc.snapping_enabled;
        self.canvas_commands(vec![Command::SetSnapping { enabled }], window, cx);
    }

    /// Adds a guide across the middle of the canvas.
    pub fn add_guide(
        &mut self,
        orientation: GuideOrientation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(canvas) = &self.canvas else {
            return;
        };
        let position = match orientation {
            GuideOrientation::Vertical => canvas.doc.width as f32 / 2.,
            GuideOrientation::Horizontal => canvas.doc.height as f32 / 2.,
        };
        self.guides_visible = true;
        self.canvas_commands(
            vec![Command::AddGuide {
                orientation,
                position,
            }],
            window,
            cx,
        );
    }

    /// The guide under the pointer, if guides are showing.
    pub fn guide_at(&self, position: Point<Pixels>) -> Option<GuideDrag> {
        let canvas = self.canvas.as_ref().filter(|_| self.guides_visible)?;
        let (rect, scale) = self.canvas_rect();
        canvas
            .doc
            .guides
            .iter()
            .filter_map(|guide| {
                let distance = match guide.orientation {
                    GuideOrientation::Vertical => {
                        f32::from(position.x - rect.origin.x) - guide.position * scale
                    }
                    GuideOrientation::Horizontal => {
                        f32::from(position.y - rect.origin.y) - guide.position * scale
                    }
                }
                .abs();
                (distance <= REACH).then_some((guide, distance))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(guide, _)| GuideDrag {
                id: guide.id,
                orientation: guide.orientation,
                position: guide.position,
            })
    }

    /// Follows the pointer with the dragged guide.
    pub fn move_guide(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let (rect, scale) = self.canvas_rect();
        let Some(drag) = self.canvas.as_mut().and_then(|c| c.guide_drag.as_mut()) else {
            return;
        };
        drag.position = match drag.orientation {
            GuideOrientation::Vertical => f32::from(position.x - rect.origin.x) / scale,
            GuideOrientation::Horizontal => f32::from(position.y - rect.origin.y) / scale,
        }
        .round();
        cx.notify();
    }

    /// Places the dragged guide, or removes it if it left the canvas.
    pub fn drop_guide(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(canvas) = &mut self.canvas else {
            return;
        };
        let Some(drag) = canvas.guide_drag.take() else {
            return;
        };
        let extent = match drag.orientation {
            GuideOrientation::Vertical => canvas.doc.width,
            GuideOrientation::Horizontal => canvas.doc.height,
        } as f32;
        let command = if (0. ..=extent).contains(&drag.position) {
            Command::MoveGuide {
                id: drag.id,
                position: drag.position,
            }
        } else {
            Command::RemoveGuide { id: drag.id }
        };
        self.canvas_commands(vec![command], window, cx);
    }

    /// Where the layer being moved lands with snapping, if snapping is on.
    pub fn drag_snap(&self) -> Option<(u64, Snapped)> {
        let canvas = self.canvas.as_ref()?;
        let drag = canvas
            .drag
            .filter(|d| d.corner.is_none() && !d.rotate && d.now != d.start)?;
        let (_, scale) = self.canvas_rect();
        let (dx, dy) = (drag.now.0 - drag.start.0, drag.now.1 - drag.start.1);
        if !canvas.doc.snapping_enabled {
            return Some((
                drag.id,
                Snapped {
                    dx,
                    dy,
                    ..Default::default()
                },
            ));
        }
        let bounds = *canvas.bounds.get(&drag.id)?;
        let others = canvas
            .doc
            .layers
            .iter()
            .filter(|l| l.visible && l.id != drag.id)
            .filter_map(|l| canvas.bounds.get(&l.id).copied());
        let (x, y) = spectrum_canvas::snap_targets(&canvas.doc, others);
        Some((
            drag.id,
            spectrum_canvas::snap_move(bounds, dx, dy, (&x, &y), REACH / scale.max(0.001)),
        ))
    }

    /// How far the layer being moved has moved: snapped exactly where it
    /// lines up with something, and by whole render pixels elsewhere.
    pub fn drag_delta(&self) -> Option<(u64, f32, f32)> {
        let density = self.canvas.as_ref()?.density;
        let (id, snapped) = self.drag_snap()?;
        let settle = |delta: f32, lined_up: bool| {
            if lined_up {
                delta
            } else {
                whole_pixels(delta, density)
            }
        };
        Some((
            id,
            settle(snapped.dx, snapped.vertical.is_some()),
            settle(snapped.dy, snapped.horizontal.is_some()),
        ))
    }

    /// Guide lines, and the lines a dragged layer snapped to, over the canvas.
    pub fn guide_overlay(&self, offset: Point<Pixels>, scale: f32, size: Size<Pixels>) -> Div {
        let mut lines: Vec<(GuideOrientation, f32, u32)> = Vec::new();
        if let Some(canvas) = self.canvas.as_ref().filter(|_| self.guides_visible) {
            let dragged = canvas.guide_drag;
            for guide in &canvas.doc.guides {
                let position = dragged
                    .filter(|d| d.id == guide.id)
                    .map_or(guide.position, |d| d.position);
                lines.push((guide.orientation, position, GUIDE));
            }
        }
        if let Some((_, snapped)) = self.drag_snap() {
            lines.extend(
                snapped
                    .vertical
                    .map(|x| (GuideOrientation::Vertical, x, SMART)),
            );
            lines.extend(
                snapped
                    .horizontal
                    .map(|y| (GuideOrientation::Horizontal, y, SMART)),
            );
        }
        div()
            .absolute()
            .left(offset.x)
            .top(offset.y)
            .w(size.width)
            .h(size.height)
            .children(lines.into_iter().map(|(orientation, position, color)| {
                let at = px(position * scale);
                match orientation {
                    GuideOrientation::Vertical => div()
                        .absolute()
                        .left(at)
                        .top_0()
                        .bottom_0()
                        .w(px(1.))
                        .bg(rgb(color)),
                    GuideOrientation::Horizontal => div()
                        .absolute()
                        .top(at)
                        .left_0()
                        .right_0()
                        .h(px(1.))
                        .bg(rgb(color)),
                }
            }))
    }

    /// The Guides group in Style > Arrange.
    pub fn guides_section(&self, cx: &mut Context<Self>) -> Div {
        let snapping = self.canvas.as_ref().is_some_and(|c| c.doc.snapping_enabled);
        let chip =
            |id: &'static str, label: &'static str, on: bool| crate::controls::chip(id, label, on);
        crate::controls::group("Guides", None)
            .gap_2()
            .child(
                div()
                    .flex()
                    .gap_1p5()
                    .child(
                        chip("add-vertical", "Add vertical", false).on_click(cx.listener(
                            |this, _, window, cx| {
                                this.add_guide(GuideOrientation::Vertical, window, cx)
                            },
                        )),
                    )
                    .child(
                        chip("add-horizontal", "Add horizontal", false).on_click(cx.listener(
                            |this, _, window, cx| {
                                this.add_guide(GuideOrientation::Horizontal, window, cx)
                            },
                        )),
                    ),
            )
            .child(
                div()
                    .flex()
                    .gap_1p5()
                    .child(
                        chip("show-guides", "Show guides", self.guides_visible)
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_guides(cx))),
                    )
                    .child(chip("snapping", "Snapping", snapping).on_click(
                        cx.listener(|this, _, window, cx| this.toggle_snapping(window, cx)),
                    )),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(FAINT))
                    .child("Drag a guide to move it, or off the canvas to remove it."),
            )
    }
}
