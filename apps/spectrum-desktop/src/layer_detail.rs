//! The part on screen of layers too large to render whole at the canvas's
//! scale. Zoomed in on a large photo, its layer keeps one image of the whole
//! layer at a capped size and, over it, the part on screen renders sharp, so
//! zooming and panning cost what the screen shows, never the whole layer.
use crate::{
    canvas_split::{LayerBounds, render_alone},
    canvas_state::CanvasState,
    layer_cache::{LayerImage, RAPID, inside, stackable, unit, unit_key, whole_density},
    workspace::Workspace,
};
use gpui::*;
use std::time::Duration;

/// How far past the screen a detail reaches, as a share of the screen's
/// size, so small pans need no new render.
const MARGIN: f32 = 0.25;
/// Quiet time after a pan before the newly shown part renders.
const AFTER_PAN: Duration = Duration::from_millis(60);

fn intersect(a: LayerBounds, b: LayerBounds) -> Option<LayerBounds> {
    let min = [a.0[0].max(b.0[0]), a.0[1].max(b.0[1])];
    let max = [a.1[0].min(b.1[0]), a.1[1].min(b.1[1])];
    (max[0] > min[0] && max[1] > min[1]).then_some((min, max))
}

/// The canvas area a layer image covers, in canvas units.
fn covered(image: &LayerImage) -> LayerBounds {
    let min = [
        image.pixel[0] / image.density,
        image.pixel[1] / image.density,
    ];
    (min, [min[0] + image.extent[0], min[1] + image.extent[1]])
}

/// Whether `image` holds all of `area`, give or take a device pixel.
pub fn covers(image: &LayerImage, area: LayerBounds) -> bool {
    let (min, max) = covered(image);
    let slack = 1. / image.density;
    min[0] <= area.0[0] + slack
        && min[1] <= area.0[1] + slack
        && max[0] >= area.1[0] - slack
        && max[1] >= area.1[1] - slack
}

/// The part of a layer, effects included, that is on screen.
pub fn seen_part(bounds: LayerBounds, reach: f32, view: LayerBounds) -> Option<LayerBounds> {
    let (min, max) = bounds;
    intersect(
        view,
        (
            [min[0] - reach, min[1] - reach],
            [max[0] + reach, max[1] + reach],
        ),
    )
}

impl Workspace {
    /// The canvas area on screen, in canvas units.
    pub fn visible_canvas_area(&self) -> Option<LayerBounds> {
        let area = *self.image.image_bounds.borrow();
        if area.size.width <= px(1.) || area.size.height <= px(1.) {
            return None;
        }
        let (rect, scale) = self.canvas_rect();
        let to_canvas = |at: Point<Pixels>| {
            [
                f32::from(at.x - rect.origin.x) / scale,
                f32::from(at.y - rect.origin.y) / scale,
            ]
        };
        Some((to_canvas(area.origin), to_canvas(area.bottom_right())))
    }

    /// Renders the part on screen of each layer too large to render whole,
    /// unless the detail it has already covers it; drops details of layers
    /// that changed, moved apart, or shrank to render whole.
    pub fn refresh_details(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let view = self.visible_canvas_area();
        let (Some(canvas), Ok(store)) = (&mut self.canvas, &self.store) else {
            return;
        };
        if !canvas.loaded || !stackable(&canvas.doc) {
            return;
        }
        let density = canvas.density;
        let doc = canvas.render_doc();
        let mut stale: Vec<u64> = canvas
            .cache
            .details
            .keys()
            .filter(|id| !doc.layers.iter().any(|l| l.id == **id))
            .copied()
            .collect();
        let mut recheck = false;
        for (index, layer) in doc.layers.iter().enumerate() {
            if inside(&doc, index) {
                continue;
            }
            let members = unit(&doc, index);
            let bounds = canvas.bounds.get(&layer.id).copied();
            let key = format!("{}|detail", unit_key(&doc, index, density, None));
            // A layer being dragged, restyled, or shown in another font
            // shows its whole image until it settles.
            let changing = canvas
                .cache
                .changed
                .get(&layer.id)
                .is_some_and(|(_, at)| at.elapsed() < RAPID);
            let settled = canvas.drag.is_none_or(|d| !members.contains(&d.id))
                && canvas
                    .font_preview
                    .as_ref()
                    .is_none_or(|(id, _)| !members.contains(id))
                && !changing;
            if changing {
                recheck = true;
            }
            let large = whole_density(bounds, &layer.style, density) < density;
            if canvas
                .cache
                .details
                .get(&layer.id)
                .is_some_and(|detail| !large || detail.key != key)
            {
                stale.push(layer.id);
            }
            if !large || !settled {
                continue;
            }
            let reach = spectrum_canvas::style_reach(&layer.style);
            let (Some(view), Some(bounds)) = (view, bounds) else {
                continue;
            };
            let Some(seen) = seen_part(bounds, reach, view) else {
                continue;
            };
            let cache = &canvas.cache;
            if cache
                .details
                .get(&layer.id)
                .is_some_and(|detail| detail.key == key && covers(detail, seen))
                || cache.detail_busy.contains_key(&layer.id)
                || cache.detail_failed.get(&layer.id) == Some(&key)
            {
                continue;
            }
            let (width, height) = (view.1[0] - view.0[0], view.1[1] - view.0[1]);
            let wide = (
                [view.0[0] - width * MARGIN, view.0[1] - height * MARGIN],
                [view.1[0] + width * MARGIN, view.1[1] + height * MARGIN],
            );
            let Some(area) = seen_part(bounds, reach, wide) else {
                continue;
            };
            canvas.cache.detail_busy.insert(layer.id, key.clone());
            let (root, render, id, layer_id) =
                (store.root.clone(), doc.clone(), canvas.id, layer.id);
            let task = cx.background_executor().spawn(async move {
                let started = std::time::Instant::now();
                let rendered = render_alone(&root, &render, &members, density, Some(area));
                crate::perf::record("canvas_detail_render", started.elapsed());
                rendered
            });
            cx.spawn_in(window, async move |this, cx| {
                let result = task.await;
                this.update_in(cx, |this, window, cx| {
                    let Some(canvas) = this.canvas.as_mut().filter(|c| c.id == id) else {
                        if let Ok((image, ..)) = result {
                            window.drop_image(image).ok();
                        }
                        return;
                    };
                    canvas.cache.detail_busy.remove(&layer_id);
                    match result {
                        Ok((image, bounds, [pixel, extent], _)) => {
                            let detail = LayerImage {
                                image,
                                pixel,
                                extent,
                                density,
                                bounds,
                                key,
                            };
                            if let Some(old) = canvas.cache.details.insert(layer_id, detail) {
                                window.drop_image(old.image).ok();
                            }
                        }
                        Err(_) => {
                            canvas.cache.detail_failed.insert(layer_id, key);
                        }
                    }
                    // The view may have moved on meanwhile.
                    this.refresh_details(window, cx);
                    cx.notify();
                })
                .ok();
            })
            .detach();
        }
        for id in stale {
            if let Some(old) = canvas.cache.details.remove(&id) {
                window.drop_image(old.image).ok();
            }
        }
        // Come back once the changing layers settle.
        if recheck {
            cx.spawn_in(window, async move |this, cx| {
                cx.background_executor()
                    .timer(RAPID + Duration::from_millis(20))
                    .await;
                this.update_in(cx, |this, window, cx| this.refresh_details(window, cx))
                    .ok();
            })
            .detach();
        }
    }

    /// Renders what a pan brought on screen once the pan pauses: layers that
    /// came into view and the parts of large ones now shown.
    pub fn refresh_details_after_pan(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(AFTER_PAN).await;
            this.update_in(cx, |this, window, cx| {
                this.refresh_layers(window, cx);
                this.refresh_composite_detail(window, cx);
            })
            .ok();
        })
        .detach();
    }

    /// For a canvas drawn as one image (one that blends layers) too large
    /// to render whole at its scale: renders the part on screen sharp over
    /// the capped whole, unless the part it has covers the screen.
    pub fn refresh_composite_detail(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let view = self.visible_canvas_area();
        let (Some(canvas), Ok(store)) = (&mut self.canvas, &self.store) else {
            return;
        };
        let density = canvas.density;
        let (version, settled) = canvas.composite_state();
        let key = format!("{version}|{}", density.to_bits());
        let wanted = canvas.loaded && !stackable(&canvas.doc) && composite_capped(canvas);
        if canvas
            .cache
            .composite
            .as_ref()
            .is_some_and(|detail| !wanted || detail.key != key)
            && let Some(old) = canvas.cache.composite.take()
        {
            window.drop_image(old.image).ok();
        }
        if wanted && !settled {
            // Come back once the canvas's render catches up with its edits.
            cx.spawn_in(window, async move |this, cx| {
                cx.background_executor()
                    .timer(RAPID + Duration::from_millis(20))
                    .await;
                this.update_in(cx, |this, window, cx| {
                    this.refresh_composite_detail(window, cx)
                })
                .ok();
            })
            .detach();
        }
        if !wanted || !settled || canvas.cache.composite_busy {
            return;
        }
        let whole = canvas_area(canvas);
        let Some(view) = view else {
            return;
        };
        let Some(seen) = intersect(view, whole) else {
            return;
        };
        if canvas
            .cache
            .composite
            .as_ref()
            .is_some_and(|detail| covers(detail, seen))
        {
            return;
        }
        let (width, height) = (view.1[0] - view.0[0], view.1[1] - view.0[1]);
        let wide = (
            [view.0[0] - width * MARGIN, view.0[1] - height * MARGIN],
            [view.1[0] + width * MARGIN, view.1[1] + height * MARGIN],
        );
        let Some(area) = intersect(wide, whole) else {
            return;
        };
        canvas.cache.composite_busy = true;
        let (root, doc, id) = (store.root.clone(), canvas.render_doc(), canvas.id);
        let task = cx.background_executor().spawn(async move {
            let started = std::time::Instant::now();
            let rendered = render_area(&root, &doc, density, area);
            crate::perf::record("canvas_detail_render", started.elapsed());
            rendered
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            this.update_in(cx, |this, window, cx| {
                let Some(canvas) = this.canvas.as_mut().filter(|c| c.id == id) else {
                    if let Ok((image, ..)) = result {
                        window.drop_image(image).ok();
                    }
                    return;
                };
                canvas.cache.composite_busy = false;
                match result {
                    Ok((image, pixel, extent)) => {
                        let detail = LayerImage {
                            image,
                            pixel,
                            extent,
                            density,
                            bounds: canvas_area(canvas),
                            key,
                        };
                        if let Some(old) = canvas.cache.composite.replace(detail) {
                            window.drop_image(old.image).ok();
                        }
                    }
                    Err(error) => return this.notify_error(error, window, cx),
                }
                // The view or the canvas may have moved on meanwhile.
                this.refresh_composite_detail(window, cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

/// The whole canvas, in canvas units.
fn canvas_area(canvas: &CanvasState) -> LayerBounds {
    (
        [0., 0.],
        [canvas.doc.width as f32, canvas.doc.height as f32],
    )
}

/// Whether a canvas drawn as one image renders whole below its scale.
fn composite_capped(canvas: &CanvasState) -> bool {
    whole_density(
        Some(canvas_area(canvas)),
        &Default::default(),
        canvas.density,
    ) < canvas.density
}

/// Whether a canvas drawn as one image shows sharp everywhere on screen:
/// it renders whole at its scale, or the part on screen covers the screen.
pub fn composite_sharp(canvas: &CanvasState, view: Option<LayerBounds>) -> bool {
    if stackable(&canvas.doc) || !composite_capped(canvas) {
        return true;
    }
    let (version, _) = canvas.composite_state();
    let key = format!("{version}|{}", canvas.density.to_bits());
    let Some(seen) = view.and_then(|view| intersect(view, canvas_area(canvas))) else {
        return true;
    };
    canvas
        .cache
        .composite
        .as_ref()
        .is_some_and(|detail| detail.key == key && covers(detail, seen))
}

/// `area` of the canvas (in canvas units) rendered at `density`, starting on
/// a whole pixel of the whole render's grid so the two line up. Returns the
/// render, its top-left pixel on that grid, and its size in canvas units.
fn render_area(
    root: &std::path::Path,
    doc: &spectrum_canvas::Document,
    density: f32,
    area: LayerBounds,
) -> anyhow::Result<(std::sync::Arc<RenderImage>, [f32; 2], [f32; 2])> {
    let pixel = [(area.0[0] * density).floor(), (area.0[1] * density).floor()];
    let end = [(area.1[0] * density).ceil(), (area.1[1] * density).ceil()];
    let origin = [pixel[0] / density, pixel[1] / density];
    let size = [
        ((end[0] - pixel[0]) / density).ceil().max(1.),
        ((end[1] - pixel[1]) / density).ceil().max(1.),
    ];
    let mut part = doc.clone();
    part.width = size[0] as u32;
    part.height = size[1] as u32;
    for layer in &mut part.layers {
        layer.transform.x -= origin[0];
        layer.transform.y -= origin[1];
    }
    let rendered = crate::canvas_state::render_at(root, &part, density)?;
    Ok((rendered.image, pixel, size))
}
