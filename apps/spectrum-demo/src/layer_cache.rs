//! Each layer rendered on its own, kept and redrawn every frame, so the canvas
//! is assembled on screen from layer images. Any layer can move at once, a
//! changed layer re-renders alone, and nothing older than the document is
//! ever shown. Canvases that blend layers with what is under them (blend
//! modes other than Normal, clipping) need the whole render instead.
use crate::{canvas_split::render_alone, workspace::Workspace};
use gpui::*;
use prism_core::{BlendMode, Document, Layer};
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

/// A layer's own render and where it was when rendered.
pub struct LayerImage {
    pub image: Arc<RenderImage>,
    /// Top-left device pixel on the canvas's pixel grid.
    pub pixel: [f32; 2],
    /// The render's size in canvas units.
    pub extent: [f32; 2],
    pub density: f32,
    /// The layer's bounds when rendered, in canvas units.
    pub bounds: ([f32; 2], [f32; 2]),
    /// The look plus the sub-pixel position and scale it was rendered at.
    key: String,
}

#[derive(Default)]
pub struct LayerCache {
    pub images: HashMap<u64, LayerImage>,
    /// Layers rendering now, and the key each render is for.
    busy: HashMap<u64, String>,
    /// Keys that failed to render, not retried until the layer changes.
    failed: HashMap<u64, String>,
    /// Each layer's latest full-quality key and when it last changed.
    changed: HashMap<u64, (String, Instant)>,
}

/// Changes closer together than this (a slider or resize drag) render as
/// drafts at half density; the full render follows once they stop.
const RAPID: Duration = Duration::from_millis(150);

/// Whether every layer can be drawn on its own and stacked on screen.
pub fn stackable(doc: &Document) -> bool {
    doc.layers
        .iter()
        .filter(|l| l.visible)
        .all(|l| matches!(l.blend_mode, BlendMode::Normal) && !l.clip_to_below)
}

/// What a layer looks like, without its name, lock, or position.
fn look(layer: &Layer) -> String {
    let mut layer = layer.clone();
    layer.transform.x = 0.;
    layer.transform.y = 0.;
    layer.name.clear();
    layer.locked = false;
    serde_json::to_string(&layer).unwrap_or_default()
}

/// The look plus what decides its pixels: the render scale and where the
/// layer falls within a device pixel, to a tenth of a pixel.
fn key(layer: &Layer, density: f32) -> String {
    let phase = |v: f32| ((v * density).rem_euclid(1.) * 10.).round() as i32 % 10;
    format!(
        "{}|{}|{}|{}",
        look(layer),
        density.to_bits(),
        phase(layer.transform.x),
        phase(layer.transform.y)
    )
}

impl Workspace {
    /// Renders layers whose look, scale, or sub-pixel position changed; the
    /// previous image stays on screen until the new one arrives.
    pub fn refresh_layers(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(canvas), Ok(store)) = (&mut self.canvas, &self.store) else {
            return;
        };
        if !canvas.loaded || !stackable(&canvas.doc) {
            return;
        }
        let density = canvas.density;
        // A layer being resized renders at the size it is being dragged to.
        let resizing = canvas.drag.and_then(|drag| {
            let transform = Self::resize_transform(canvas, drag)
                .map(|(transform, _)| transform)
                .or_else(|| Self::rotate_transform(canvas, drag))?;
            let mut layer = canvas.doc.layers.iter().find(|l| l.id == drag.id)?.clone();
            layer.transform = transform;
            Some(layer)
        });
        let render_doc = canvas.render_doc();
        let cache = &mut canvas.cache;
        cache
            .images
            .retain(|id, _| canvas.doc.layers.iter().any(|l| l.id == *id));
        for layer in &render_doc.layers {
            let layer = resizing
                .as_ref()
                .filter(|r| r.id == layer.id)
                .unwrap_or(layer);
            // A font being previewed on this layer is part of its look.
            let preview = canvas
                .font_preview
                .as_ref()
                .filter(|(previewed, _)| *previewed == layer.id)
                .map(|(_, path)| path.clone());
            let font = preview
                .as_ref()
                .map_or(String::new(), |path| format!("|font:{}", path.display()));
            let full = format!("{}{font}", key(layer, density));
            let current = cache.images.get(&layer.id).map(|i| i.key.as_str());
            if current == Some(full.as_str()) {
                continue;
            }
            // A change soon after the last one renders as a draft.
            let now = Instant::now();
            let rapid = match cache.changed.get(&layer.id) {
                Some((last, at)) if *last == full => at.elapsed() < RAPID,
                Some((_, at)) => {
                    let rapid = at.elapsed() < RAPID;
                    cache.changed.insert(layer.id, (full.clone(), now));
                    rapid
                }
                None => {
                    cache.changed.insert(layer.id, (full.clone(), now));
                    false
                }
            };
            let render_density = if rapid {
                (density * 0.5).max(0.5)
            } else {
                density
            };
            let wanted = if rapid {
                format!("{}{font}|draft", key(layer, render_density))
            } else {
                full
            };
            if current == Some(wanted.as_str())
                || cache.busy.contains_key(&layer.id)
                || cache.failed.get(&layer.id) == Some(&wanted)
            {
                continue;
            }
            if rapid {
                // Come back for the full render once the changes stop.
                cx.spawn_in(window, async move |this, cx| {
                    cx.background_executor()
                        .timer(RAPID + Duration::from_millis(20))
                        .await;
                    this.update_in(cx, |this, window, cx| this.refresh_layers(window, cx))
                        .ok();
                })
                .detach();
            }
            cache.busy.insert(layer.id, wanted.clone());
            let (root, mut doc, id, layer_id) =
                (store.root.clone(), render_doc.clone(), canvas.id, layer.id);
            if let Some(slot) = doc.layers.iter_mut().find(|l| l.id == layer_id) {
                *slot = layer.clone();
            }
            let at = (layer.transform.x, layer.transform.y);
            let previewed = preview.clone();
            let task = cx.background_executor().spawn(async move {
                let doc = match preview {
                    Some(path) => crate::font_browser::with_font(&doc, layer_id, &path)?,
                    None => doc,
                };
                render_alone(&root, &doc, layer_id, render_density)
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
                    canvas.cache.busy.remove(&layer_id);
                    let (image, bounds, [pixel, extent]) = match result {
                        Ok(rendered) => rendered,
                        Err(error) => {
                            canvas.cache.failed.insert(layer_id, wanted);
                            // A font that cannot be embedded is marked in the
                            // browser instead of interrupting with an error.
                            if let Some(path) = previewed {
                                this.font_blocked.insert(path);
                                return cx.notify();
                            }
                            return this.notify_error(error, window, cx);
                        }
                    };
                    let entry = LayerImage {
                        image,
                        pixel,
                        extent,
                        density: render_density,
                        bounds,
                        key: wanted,
                    };
                    if let Some(old) = canvas.cache.images.insert(layer_id, entry) {
                        window.drop_image(old.image).ok();
                    }
                    // Bounds follow the layer if it moved meanwhile.
                    let dragging = canvas.drag.is_some_and(|d| d.id == layer_id);
                    if let Some(now) = canvas.doc.layers.iter().find(|l| l.id == layer_id)
                        && !dragging
                    {
                        let (dx, dy) = (now.transform.x - at.0, now.transform.y - at.1);
                        canvas.bounds.insert(
                            layer_id,
                            (
                                [bounds.0[0] + dx, bounds.0[1] + dy],
                                [bounds.1[0] + dx, bounds.1[1] + dy],
                            ),
                        );
                    }
                    this.refresh_layers(window, cx);
                    cx.notify();
                })
                .ok();
            })
            .detach();
        }
    }

    /// Re-renders every layer, keeping their images until replaced, after
    /// something outside the document changed (a shared image's edits).
    pub fn invalidate_layers(&mut self) {
        if let Some(canvas) = &mut self.canvas {
            for image in canvas.cache.images.values_mut() {
                image.key.clear();
            }
        }
    }

    /// Frees every layer image.
    pub fn drop_layers(cache: &mut LayerCache, window: &mut Window) {
        for (_, image) in cache.images.drain() {
            window.drop_image(image.image).ok();
        }
        cache.busy.clear();
    }
}
