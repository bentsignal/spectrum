//! Zooming and panning the open image, as on the canvas: two-finger
//! gestures, ⌘= ⌘− ⌘0 ⌘⌥0, and the header's zoom. 100% shows each of the
//! edited image's pixels on one screen pixel. Zoomed in past the preview's
//! own pixels, the part on screen renders from the full-resolution photo and
//! shows sharp over the preview.
use crate::{
    workspace::{Mode, Open, Workspace},
    zoom::ZoomAnim,
};
use gpui::*;
use rayon::prelude::*;
use spectrum_image::{Adjustments, render::PixelRegion};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

/// The zooms the keys step through, in screen pixels per image pixel.
const STEPS: [f32; 12] = [
    0.0625, 0.125, 0.1667, 0.25, 0.3333, 0.5, 0.6667, 1., 2., 4., 8., 16.,
];
const MAX_ZOOM: f32 = 16.;
/// Space around the fitted image, in screen pixels: as the image view pads it.
const PAD: f32 = 32.;
const PAD_BOTTOM: f32 = 40.;
/// How far past the screen a sharp part reaches, as a share of the screen.
const MARGIN: f32 = 0.25;
/// Quiet time after a zoom or pan before the part on screen renders sharp.
const AFTER_MOVE: Duration = Duration::from_millis(60);

/// The open image's full-resolution photo, decoded when first zoomed in.
pub struct FullPhoto {
    pixels: image::DynamicImage,
}

/// The sharp part on screen: the edited image's pixels in `region` (in
/// full-resolution pixels of the edited image) at `zoom`, for `adjustments`.
pub struct ImageDetail {
    pub image: Arc<RenderImage>,
    region: PixelRegion,
    zoom: f32,
    adjustments: Adjustments,
}

#[derive(Default)]
pub struct ImageView {
    /// Screen pixels per pixel of the edited image; `None` fits it.
    pub zoom: Option<f32>,
    /// Where the image's middle is from the view's middle, in points.
    pub pan: (f32, f32),
    anim: Option<ZoomAnim>,
    /// A drag panning the image: where it started and the pan then.
    dragging: Option<(Point<Pixels>, (f32, f32))>,
    pub detail: Option<ImageDetail>,
    detail_busy: bool,
    full: Option<Arc<FullPhoto>>,
    full_loading: bool,
}

impl ImageView {
    /// Forgets the zoom and the full-resolution photo of the image shown
    /// before, freeing the sharp part's pixels.
    pub fn reset(&mut self) {
        if let Some(detail) = self.detail.take() {
            crate::retire::drop_later(detail.image);
        }
        *self = Self::default();
    }
}

impl Workspace {
    /// Whether the open image can zoom: shown alone, not cropping or comparing.
    pub fn image_zooms(&self) -> bool {
        matches!(self.open, Open::Image(_))
            && !matches!(self.mode, Mode::Crop | Mode::History)
            && !self.image.compare
    }

    /// The edited image's full-resolution size.
    fn image_size(&self) -> Option<(f32, f32)> {
        let adjustments = self.image.adjust.clone();
        let (width, height) = match &self.image.view.full {
            Some(full) => (full.pixels.width(), full.pixels.height()),
            None => {
                let photo = self.image.photo_info.as_ref()?;
                (photo.width, photo.height)
            }
        };
        spectrum_image::render::adjusted_image_dimensions(width, height, &adjustments)
            .map(|(w, h)| (w as f32, h as f32))
    }

    /// The box the image fits in: the view less its padding. The crop view
    /// fits the whole frame in the same box, so switching moves nothing.
    pub fn image_box(&self) -> Bounds<Pixels> {
        let area = *self.image.image_bounds.borrow();
        Bounds::new(
            area.origin + point(px(PAD), px(PAD)),
            size(
                (area.size.width - px(2. * PAD)).max(px(1.)),
                (area.size.height - px(PAD + PAD_BOTTOM)).max(px(1.)),
            ),
        )
    }

    /// The zoom that fits the image, in screen pixels per image pixel.
    pub fn image_fit_zoom(&self, window: &Window) -> Option<f32> {
        let (width, height) = self.image_size()?;
        let fit = self.image_box();
        let points = (f32::from(fit.size.width) / width).min(f32::from(fit.size.height) / height);
        Some(points * window.scale_factor())
    }

    /// Where the image is drawn once any zoom in progress arrives, and its
    /// zoom in screen pixels per image pixel.
    fn image_rect_target(&self, window: &Window) -> Option<(Bounds<Pixels>, f32)> {
        let (width, height) = self.image_size()?;
        let fit = self.image_fit_zoom(window)?;
        let view = &self.image.view;
        let zoom = view.zoom.unwrap_or(fit);
        let scale = zoom / window.scale_factor();
        let shown = size(px(width * scale), px(height * scale));
        let center = self.image_box().center() + point(px(view.pan.0), px(view.pan.1));
        Some((
            Bounds::new(center - point(shown.width / 2., shown.height / 2.), shown),
            zoom,
        ))
    }

    /// Where the image is drawn now, on the way while a zoom moves.
    pub fn image_rect(&self, window: &Window) -> Option<(Bounds<Pixels>, f32)> {
        let target = self.image_rect_target(window)?;
        match self.image.view.anim {
            Some(anim) => Some(anim.at(target, self.image_box())),
            None => Some(target),
        }
    }

    /// Keeps at least part of the image on screen.
    fn clamp_image_pan(&mut self, window: &Window) {
        let Some((rect, _)) = self.image_rect_target(window) else {
            return;
        };
        let area = self.image_box();
        let reach =
            |area: Pixels, shown: Pixels| ((f32::from(area) + f32::from(shown)) / 2. - 80.).max(0.);
        let (rx, ry) = (
            reach(area.size.width, rect.size.width),
            reach(area.size.height, rect.size.height),
        );
        let pan = &mut self.image.view.pan;
        *pan = (pan.0.clamp(-rx, rx), pan.1.clamp(-ry, ry));
    }

    /// Zooms to `zoom` (fit for `None`), keeping the image point under
    /// `about` where it is; smoothly when `animate`.
    pub fn image_zoom_to(
        &mut self,
        zoom: Option<f32>,
        about: Option<Point<Pixels>>,
        animate: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (Some((rect, current)), Some(fit)) =
            (self.image_rect_target(window), self.image_fit_zoom(window))
        else {
            return;
        };
        let from = self.image_rect(window);
        let view = &mut self.image.view;
        match zoom.map(|zoom| zoom.clamp(fit.min(1.), MAX_ZOOM)) {
            Some(zoom) if zoom > fit * 1.001 => {
                let about = about.unwrap_or(self.image_box().center());
                // Keep the image point under `about` in place.
                let k = zoom / current;
                let origin = about - (about - rect.origin) * k;
                let shown = size(rect.size.width * k, rect.size.height * k);
                let center = origin + point(shown.width / 2., shown.height / 2.);
                let middle = self.image_box().center();
                let view = &mut self.image.view;
                view.zoom = Some(zoom);
                view.pan = (
                    f32::from(center.x - middle.x),
                    f32::from(center.y - middle.y),
                );
            }
            _ => {
                view.zoom = None;
                view.pan = (0., 0.);
            }
        }
        self.clamp_image_pan(window);
        // Rendering keeps an animation going (`step_image_animation`).
        self.image.view.anim = from.filter(|_| animate).map(ZoomAnim::new);
        self.refresh_image_detail_soon(window, cx);
        cx.notify();
    }

    /// Steps to the next zoom in (`1`) or out (`-1`).
    pub fn image_zoom_step(&mut self, direction: i32, window: &mut Window, cx: &mut Context<Self>) {
        let (Some((_, now)), Some(fit)) =
            (self.image_rect_target(window), self.image_fit_zoom(window))
        else {
            return;
        };
        let steps = STEPS.iter().copied().chain(std::iter::once(fit));
        let next = if direction > 0 {
            steps.filter(|step| *step > now * 1.01).reduce(f32::min)
        } else {
            steps.filter(|step| *step < now * 0.99).reduce(f32::max)
        };
        if let Some(zoom) = next {
            self.image_zoom_to(Some(zoom), None, true, window, cx);
        }
    }

    /// Two-finger gestures and the scroll wheel over the image.
    pub fn image_scroll(
        &mut self,
        event: &ScrollWheelEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.stop_propagation();
        let Some(scroll) = self.gestures.read(event) else {
            return;
        };
        match scroll.gesture {
            crate::gesture::Gesture::Pan => {
                if self.image.view.zoom.is_none() {
                    return;
                }
                let view = &mut self.image.view;
                view.anim = None;
                view.pan = (view.pan.0 + scroll.dx, view.pan.1 + scroll.dy);
                self.clamp_image_pan(window);
                self.refresh_image_detail_soon(window, cx);
                cx.notify();
            }
            crate::gesture::Gesture::Zoom => {
                let Some((_, now)) = self.image_rect_target(window) else {
                    return;
                };
                self.image.view.anim = None;
                let zoom = now * scroll.factor;
                self.image_zoom_to(Some(zoom), Some(event.position), false, window, cx);
            }
        }
    }

    /// Dragging the zoomed-in image pans it.
    pub fn image_press(&mut self, position: Point<Pixels>) {
        let view = &mut self.image.view;
        if view.zoom.is_some() {
            view.dragging = Some((position, view.pan));
        }
    }

    pub fn image_drag(
        &mut self,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((start, pan)) = self.image.view.dragging else {
            return;
        };
        let moved = position - start;
        self.image.view.pan = (pan.0 + f32::from(moved.x), pan.1 + f32::from(moved.y));
        self.clamp_image_pan(window);
        self.refresh_image_detail_soon(window, cx);
        cx.notify();
    }

    pub fn image_release(&mut self) {
        self.image.view.dragging = None;
    }

    /// Ends a finished zoom animation.
    pub fn step_image_animation(&mut self, window: &mut Window) {
        if let Some(anim) = self.image.view.anim {
            if anim.done() {
                self.image.view.anim = None;
            } else {
                window.request_animation_frame();
            }
        }
    }

    /// Renders the part on screen sharp once moving pauses.
    fn refresh_image_detail_soon(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(AFTER_MOVE).await;
            this.update_in(cx, |this, window, cx| this.refresh_image_detail(window, cx))
                .ok();
        })
        .detach();
    }

    /// The sharp part on screen, when it shows for the current edits.
    pub fn image_detail_shown(
        &self,
        window: &Window,
    ) -> Option<(Arc<RenderImage>, Bounds<Pixels>)> {
        let detail = self.image.view.detail.as_ref()?;
        if detail.adjustments != self.image.adjust || !self.image_zooms() {
            return None;
        }
        let (rect, _) = self.image_rect(window)?;
        let (width, _) = self.image_size()?;
        let per_pixel = f32::from(rect.size.width) / width;
        let region = detail.region;
        Some((
            detail.image.clone(),
            Bounds::new(
                rect.origin
                    + point(
                        px(region.x as f32 * per_pixel),
                        px(region.y as f32 * per_pixel),
                    ),
                size(
                    px(region.width as f32 * per_pixel),
                    px(region.height as f32 * per_pixel),
                ),
            ),
        ))
    }

    /// Whether the image shows sharp everywhere on screen: the preview
    /// holds enough pixels, or the sharp part covers the screen.
    pub fn image_detail_ready(&self, window: &Window) -> bool {
        match self.wanted_detail(window) {
            None => true,
            Some((region, zoom)) => self.image.view.detail.as_ref().is_some_and(|detail| {
                detail.adjustments == self.image.adjust
                    && (detail.zoom - zoom).abs() <= zoom * 0.01
                    && covers(detail.region, region)
            }),
        }
    }

    /// The part on screen in full-resolution image pixels, and the zoom,
    /// when the preview shows it with fewer pixels than the screen has.
    fn wanted_detail(&self, window: &Window) -> Option<(PixelRegion, f32)> {
        if !self.image_zooms() {
            return None;
        }
        let zoom = self.image.view.zoom?;
        let (width, height) = self.image_size()?;
        let preview = self.image.preview.as_ref()?.image.as_ref()?.size(0);
        let preview_zoom = preview.width.0 as f32 / width;
        if zoom <= preview_zoom * 1.05 {
            return None;
        }
        let (rect, _) = self.image_rect_target(window)?;
        let area = self.image_box();
        let per_pixel = f32::from(rect.size.width) / width;
        let to_image = |at: Point<Pixels>| {
            (
                (f32::from(at.x - rect.origin.x) / per_pixel).clamp(0., width),
                (f32::from(at.y - rect.origin.y) / per_pixel).clamp(0., height),
            )
        };
        let (x0, y0) = to_image(area.origin);
        let (x1, y1) = to_image(area.bottom_right());
        (x1 > x0 && y1 > y0).then(|| {
            (
                PixelRegion {
                    x: x0.floor() as u32,
                    y: y0.floor() as u32,
                    width: (x1.ceil() - x0.floor()).max(1.) as u32,
                    height: (y1.ceil() - y0.floor()).max(1.) as u32,
                },
                zoom,
            )
        })
    }

    /// Renders the part on screen from the full-resolution photo when the
    /// preview has too few pixels for the zoom, once edits are drawn.
    pub fn refresh_image_detail(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((seen, zoom)) = self.wanted_detail(window) else {
            return;
        };
        if self.image_detail_ready(window) || self.image.view.detail_busy {
            return;
        }
        // Edits in progress show from the preview; the sharp part follows.
        let settled = self
            .image
            .preview
            .as_ref()
            .is_some_and(|p| p.image.is_some())
            && self.image_idle_view();
        if !settled {
            return;
        }
        let (Some(id), Ok(store)) = (self.open_image(), &self.store) else {
            return;
        };
        let Some(full) = self.image.view.full.clone() else {
            if !self.image.view.full_loading {
                self.image.view.full_loading = true;
                let root = store.root.clone();
                let task = cx.background_executor().spawn(async move {
                    let photo = spectrum_assets::Service::open(&root)?.image(id)?;
                    spectrum_image::engine::decode_photo(&photo, None)
                });
                cx.spawn_in(window, async move |this, cx| {
                    let decoded = task.await;
                    this.update_in(cx, |this, window, cx| {
                        if this.open_image() != Some(id) {
                            return;
                        }
                        this.image.view.full_loading = false;
                        match decoded {
                            Ok(pixels) => {
                                this.image.view.full = Some(Arc::new(FullPhoto { pixels }));
                                this.refresh_image_detail(window, cx);
                            }
                            Err(error) => this.notify_error(error, window, cx),
                        }
                    })
                    .ok();
                })
                .detach();
            }
            return;
        };
        let (width, height) = self.image_size().unwrap_or_default();
        let (sx, sy) = (seen.width as f32 * MARGIN, seen.height as f32 * MARGIN);
        let x0 = (seen.x as f32 - sx).max(0.);
        let y0 = (seen.y as f32 - sy).max(0.);
        let region = PixelRegion {
            x: x0 as u32,
            y: y0 as u32,
            width: ((seen.x + seen.width) as f32 + sx).min(width) as u32 - x0 as u32,
            height: ((seen.y + seen.height) as f32 + sy).min(height) as u32 - y0 as u32,
        };
        let adjustments = self.image.adjust.clone();
        self.image.view.detail_busy = true;
        let task = cx.background_executor().spawn({
            let adjustments = adjustments.clone();
            async move {
                let started = Instant::now();
                let rendered = render_region(&full, adjustments, region, zoom);
                crate::perf::record("image_detail_render", started.elapsed());
                rendered
            }
        });
        cx.spawn_in(window, async move |this, cx| {
            let rendered = task.await;
            this.update_in(cx, |this, window, cx| {
                if this.open_image() != Some(id) {
                    if let Ok(image) = rendered {
                        crate::retire::drop_later(image);
                    }
                    return;
                }
                let view = &mut this.image.view;
                view.detail_busy = false;
                match rendered {
                    Ok(image) => {
                        let detail = ImageDetail {
                            image,
                            region,
                            zoom,
                            adjustments,
                        };
                        if let Some(old) = view.detail.replace(detail) {
                            crate::retire::drop_later(old.image);
                        }
                    }
                    Err(error) => this.notify_error(error, window, cx),
                }
                // The view or the edits may have moved on meanwhile.
                this.refresh_image_detail(window, cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

fn covers(outer: PixelRegion, inner: PixelRegion) -> bool {
    outer.x <= inner.x
        && outer.y <= inner.y
        && outer.x + outer.width >= inner.x + inner.width
        && outer.y + outer.height >= inner.y + inner.height
}

/// The edited image's `region` at full resolution, from `full`, shrunk to
/// `zoom` when that is below one screen pixel per image pixel.
fn render_region(
    full: &FullPhoto,
    adjustments: Adjustments,
    region: PixelRegion,
    zoom: f32,
) -> anyhow::Result<Arc<RenderImage>> {
    let source = &full.pixels;
    let pixels = spectrum_image::render::render_image_region_at_source_resolution(
        source.width(),
        source.height(),
        adjustments,
        region,
        |wanted: PixelRegion| {
            Ok::<_, std::convert::Infallible>(spectrum_image::render::to_rgba(source.crop_imm(
                wanted.x,
                wanted.y,
                wanted.width,
                wanted.height,
            )))
        },
    )
    .map_err(|error| anyhow::anyhow!("{error}"))?;
    let pixels = if zoom < 1. {
        let wanted = ((region.width.max(region.height) as f32) * zoom).ceil() as u32;
        spectrum_image::downscale(&image::DynamicImage::ImageRgba8(pixels), wanted.max(1))
            .into_rgba8()
    } else {
        pixels
    };
    let mut bgra = pixels;
    bgra.par_chunks_mut(4 * 4096).for_each(|chunk| {
        for pixel in chunk.as_chunks_mut::<4>().0 {
            pixel.swap(0, 2);
        }
    });
    Ok(Arc::new(RenderImage::new(vec![image::Frame::new(bgra)])))
}
