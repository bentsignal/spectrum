//! Crop mode: the whole frame fills the main area with a crop box to drag;
//! the sidebar holds aspect, rotation, flips, and straightening.
use crate::{
    controls::{chip, group, slider_row},
    theme::*,
    workspace::Workspace,
};
use gpui::{prelude::*, *};
use gpui_component::{
    IconName, Sizable,
    button::{Button, ButtonVariants},
};
use spectrum_image::{Adjustments, CropRect};

pub const ASPECTS: [(&str, Option<f32>); 5] = [
    ("Free", None),
    ("1:1", Some(1.)),
    ("4:5", Some(0.8)),
    ("3:2", Some(1.5)),
    ("16:9", Some(16. / 9.)),
];
/// Handle size, in pixels, for grabbing a corner of the crop box.
const HANDLE: f32 = 14.;

#[derive(Clone, Copy)]
pub enum Grip {
    Move,
    Corner(bool, bool),
}

#[derive(Clone, Copy)]
pub struct CropDrag {
    pub grip: Grip,
    pub start: (f32, f32),
    pub rect: CropRect,
}

impl Workspace {
    /// Pixel size of the uncropped frame the preview shows while cropping.
    fn frame_size(&self) -> (u32, u32) {
        self.image
            .preview
            .as_ref()
            .and_then(|p| p.image.as_ref())
            .map_or((1, 1), |image| {
                let size = image.size(0);
                (size.width.0.max(1) as u32, size.height.0.max(1) as u32)
            })
    }

    /// Where the frame is drawn: fitted and centered in the main area.
    fn frame_rect(&self) -> Bounds<Pixels> {
        let area = *self.image.image_bounds.borrow();
        let (w, h) = self.frame_size();
        let scale =
            (f32::from(area.size.width) / w as f32).min(f32::from(area.size.height) / h as f32);
        let size = size(px(w as f32 * scale), px(h as f32 * scale));
        Bounds::new(
            area.origin
                + point(
                    (area.size.width - size.width) / 2.,
                    (area.size.height - size.height) / 2.,
                ),
            size,
        )
    }

    fn to_frame(&self, position: Point<Pixels>) -> (f32, f32) {
        let rect = self.frame_rect();
        let x = f32::from(position.x - rect.origin.x) / f32::from(rect.size.width).max(1.);
        let y = f32::from(position.y - rect.origin.y) / f32::from(rect.size.height).max(1.);
        (x.clamp(0., 1.), y.clamp(0., 1.))
    }

    fn crop_rect(&self) -> CropRect {
        self.image.adjust.crop.unwrap_or_default()
    }

    /// Height over width of the frame in pixels, to hold aspect ratios.
    fn frame_ratio(&self) -> f32 {
        let (w, h) = self.frame_size();
        h as f32 / w.max(1) as f32
    }

    fn crop_down(&mut self, event: &MouseDownEvent, cx: &mut Context<Self>) {
        let rect = self.crop_rect();
        let frame = self.frame_rect();
        let (fw, fh) = (f32::from(frame.size.width), f32::from(frame.size.height));
        let (x, y) = self.to_frame(event.position);
        let near = |a: f32, b: f32, span: f32| ((a - b) * span).abs() <= HANDLE;
        let grip = [(false, false), (true, false), (false, true), (true, true)]
            .into_iter()
            .find(|(right, bottom)| {
                let cx_ = if *right { rect.x + rect.width } else { rect.x };
                let cy = if *bottom {
                    rect.y + rect.height
                } else {
                    rect.y
                };
                near(x, cx_, fw) && near(y, cy, fh)
            })
            .map(|(r, b)| Grip::Corner(r, b))
            .unwrap_or(Grip::Move);
        self.image.crop_drag = Some(CropDrag {
            grip,
            start: (x, y),
            rect,
        });
        cx.notify();
    }

    fn crop_move(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        let Some(drag) = self.image.crop_drag else {
            return;
        };
        if event.pressed_button != Some(MouseButton::Left) {
            self.image.crop_drag = None;
            return;
        }
        let (x, y) = self.to_frame(event.position);
        let (dx, dy) = (x - drag.start.0, y - drag.start.1);
        let r = drag.rect;
        let mut next = match drag.grip {
            Grip::Move => CropRect {
                x: (r.x + dx).clamp(0., 1. - r.width),
                y: (r.y + dy).clamp(0., 1. - r.height),
                ..r
            },
            Grip::Corner(right, bottom) => {
                let (mut left, mut top) = (r.x, r.y);
                let (mut rx, mut by) = (r.x + r.width, r.y + r.height);
                if right {
                    rx = (rx + dx).clamp(left + 0.02, 1.)
                } else {
                    left = (left + dx).clamp(0., rx - 0.02)
                }
                if bottom {
                    by = (by + dy).clamp(top + 0.02, 1.)
                } else {
                    top = (top + dy).clamp(0., by - 0.02)
                }
                CropRect {
                    x: left,
                    y: top,
                    width: rx - left,
                    height: by - top,
                }
            }
        };
        if let (Some(aspect), Grip::Corner(_, bottom)) =
            (ASPECTS[self.image.crop_aspect].1, drag.grip)
        {
            // Width drives height; in normalized units the frame's own shape matters.
            let height = (next.width / aspect / self.frame_ratio()).min(1.);
            if !bottom {
                next.y = (next.y + next.height - height).max(0.);
            }
            next.height = height.min(1. - next.y);
        }
        self.image.adjust.crop = Some(next.sanitized());
        cx.notify();
    }

    fn crop_up(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.image.crop_drag.take().is_some() {
            self.schedule_color_edit(window, cx);
        }
    }

    /// Applies an aspect preset as the largest centered box of that shape.
    fn set_aspect(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.image.crop_aspect = index;
        if let Some(aspect) = ASPECTS[index].1 {
            let ratio = self.frame_ratio();
            let (mut w, mut h) = (1., 1. / aspect / ratio);
            if h > 1. {
                w /= h;
                h = 1.;
            }
            self.image.adjust.crop = Some(CropRect {
                x: (1. - w) / 2.,
                y: (1. - h) / 2.,
                width: w,
                height: h,
            });
            self.schedule_color_edit(window, cx);
        }
        cx.notify();
    }

    fn geometry(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut Adjustments),
    ) {
        f(&mut self.image.adjust);
        // A new frame shape invalidates the box.
        self.image.adjust.crop = None;
        self.sync_color_sliders(window, cx);
        self.schedule_color_edit(window, cx);
    }

    pub fn crop_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let aspects = ASPECTS.iter().enumerate().map(|(index, (name, _))| {
            chip(name, name, index == self.image.crop_aspect).on_click(
                cx.listener(move |this, _, window, cx| this.set_aspect(index, window, cx)),
            )
        });
        let action = |id: &'static str, icon: IconName, label: &'static str| {
            Button::new(id).small().icon(icon).label(label).flex_1()
        };
        let reset = Button::new("reset-crop")
            .ghost()
            .xsmall()
            .label("Reset")
            .text_color(rgb(MUTED))
            .on_click(cx.listener(|this, _, window, cx| {
                this.image.crop_aspect = 0;
                this.geometry(window, cx, |a| {
                    a.rotation = 0;
                    a.flip_horizontal = false;
                    a.flip_vertical = false;
                    a.straighten = 0.;
                })
            }))
            .into_any_element();
        div()
            .flex()
            .flex_col()
            .gap_7()
            .child(
                group("Aspect", None).child(div().flex().flex_wrap().gap_1p5().children(aspects)),
            )
            .child(
                group("Transform", Some(reset))
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(action("rotate-left", IconName::Undo2, "Rotate").on_click(
                                cx.listener(|this, _, window, cx| {
                                    this.geometry(window, cx, |a| {
                                        a.rotation = (a.rotation + 270) % 360
                                    })
                                }),
                            ))
                            .child(action("rotate-right", IconName::Redo2, "Rotate").on_click(
                                cx.listener(|this, _, window, cx| {
                                    this.geometry(window, cx, |a| {
                                        a.rotation = (a.rotation + 90) % 360
                                    })
                                }),
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(action("flip-h", IconName::ArrowRight, "Flip").on_click(
                                cx.listener(|this, _, window, cx| {
                                    this.geometry(window, cx, |a| {
                                        a.flip_horizontal = !a.flip_horizontal
                                    })
                                }),
                            ))
                            .child(action("flip-v", IconName::ArrowDown, "Flip").on_click(
                                cx.listener(|this, _, window, cx| {
                                    this.geometry(window, cx, |a| {
                                        a.flip_vertical = !a.flip_vertical
                                    })
                                }),
                            )),
                    )
                    .child(slider_row(
                        "Straighten",
                        format!("{:.1}°", self.image.straighten.read(cx).value().start()),
                        &self.image.straighten,
                    )),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(FAINT))
                    .child("Drag the box to move it, or a corner to resize it."),
            )
    }

    /// The whole frame with the crop box over it.
    pub fn crop_view(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let bounds_slot = self.image.image_bounds.clone();
        let image = self.image.preview.as_ref().and_then(|p| p.image.clone());
        let frame = self.frame_rect();
        let area = *self.image.image_bounds.borrow();
        let r = self.crop_rect();
        let (fx, fy) = (
            frame.origin.x - area.origin.x,
            frame.origin.y - area.origin.y,
        );
        let (fw, fh) = (frame.size.width, frame.size.height);
        let box_ = Bounds::new(
            point(fx + fw * r.x, fy + fh * r.y),
            size(fw * r.width, fh * r.height),
        );
        let shade = hsla(0., 0., 0., 0.55);
        let dim = |x: Pixels, y: Pixels, w: Pixels, h: Pixels| {
            div().absolute().left(x).top(y).w(w).h(h).bg(shade)
        };
        let handle = |x: Pixels, y: Pixels| {
            div()
                .absolute()
                .left(x - px(5.))
                .top(y - px(5.))
                .size(px(10.))
                .rounded_sm()
                .bg(rgb(0xf4f4f4))
        };
        let (bx, by, bw, bh) = (
            box_.origin.x,
            box_.origin.y,
            box_.size.width,
            box_.size.height,
        );
        div().size_full().p_8().pb(px(40.)).child(
            div()
                .id("crop-area")
                .relative()
                .size_full()
                .child(
                    canvas(
                        move |bounds, _, _| *bounds_slot.borrow_mut() = bounds,
                        |_, _, _, _| {},
                    )
                    .absolute()
                    .size_full(),
                )
                .children(image.map(|image| {
                    img(image)
                        .absolute()
                        .left(fx)
                        .top(fy)
                        .w(fw)
                        .h(fh)
                        .object_fit(ObjectFit::Fill)
                }))
                .child(dim(fx, fy, fw, by - fy))
                .child(dim(fx, by + bh, fw, fy + fh - by - bh))
                .child(dim(fx, by, bx - fx, bh))
                .child(dim(bx + bw, by, fx + fw - bx - bw, bh))
                .child(
                    div()
                        .absolute()
                        .left(bx)
                        .top(by)
                        .w(bw)
                        .h(bh)
                        .border_1()
                        .border_color(hsla(0., 0., 1., 0.85)),
                )
                .child(handle(bx, by))
                .child(handle(bx + bw, by))
                .child(handle(bx, by + bh))
                .child(handle(bx + bw, by + bh))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, event, _, cx| this.crop_down(event, cx)),
                )
                .on_mouse_move(cx.listener(|this, event, _, cx| this.crop_move(event, cx)))
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(|this, _, window, cx| this.crop_up(window, cx)),
                )
                .on_mouse_up_out(
                    MouseButton::Left,
                    cx.listener(|this, _, window, cx| this.crop_up(window, cx)),
                ),
        )
    }
}
