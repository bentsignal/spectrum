//! Each layer rendered on its own, kept and redrawn every frame, so the canvas
//! is assembled on screen from layer images. Any layer can move at once, a
//! changed layer re-renders alone, and nothing older than the document is
//! ever shown. Canvases that blend layers with what is under them (blend
//! modes other than Normal, clipping) need the whole render instead.
use crate::{
    canvas_split::{LayerBounds, render_alone},
    workspace::Workspace,
};
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
/// drafts; the full render follows once they stop.
const RAPID: Duration = Duration::from_millis(150);

/// Drafts render at most this many device pixels, so they stay fast for
/// large images while small layers such as text keep their full sharpness.
const DRAFT_PIXELS: f32 = 1_200_000.;

/// The density for a draft of a layer with these bounds and styles.
fn draft_density(bounds: Option<LayerBounds>, style: &prism_core::LayerStyle, density: f32) -> f32 {
    let Some((min, max)) = bounds else {
        return (density * 0.5).max(0.25);
    };
    let reach = prism_core::style_reach(style);
    let area = (max[0] - min[0] + 2. * reach) * (max[1] - min[1] + 2. * reach) * density * density;
    let factor = (DRAFT_PIXELS / area.max(1.)).sqrt().clamp(0.25, 1.);
    density * factor
}

impl LayerCache {
    /// Moves every layer image by a canvas-space offset at once, as when the
    /// canvas is cropped and every layer shifts with its origin.
    pub fn shift(&mut self, dx: f32, dy: f32) {
        for image in self.images.values_mut() {
            image.pixel = [
                image.pixel[0] + dx * image.density,
                image.pixel[1] + dy * image.density,
            ];
            let (min, max) = image.bounds;
            image.bounds = ([min[0] + dx, min[1] + dy], [max[0] + dx, max[1] + dy]);
        }
    }
}

/// Whether the canvas can be drawn from layer images stacked on screen:
/// true unless a layer blends with what is under it. A layer and the layers
/// inside it (clipped to it) render together as one image.
pub fn stackable(doc: &Document) -> bool {
    doc.layers
        .iter()
        .filter(|l| l.visible)
        .all(|l| matches!(l.blend_mode, BlendMode::Normal))
}

/// Whether the layer at `index` is drawn inside the one below it.
pub fn inside(doc: &Document, index: usize) -> bool {
    index > 0 && doc.layers[index].clip_to_below
}

/// The layers drawn as one image with the layer at `index`: it and the
/// layers inside it, bottom first.
pub fn unit(doc: &Document, index: usize) -> Vec<u64> {
    std::iter::once(doc.layers[index].id)
        .chain(
            doc.layers[index + 1..]
                .iter()
                .take_while(|l| l.clip_to_below)
                .map(|l| l.id),
        )
        .collect()
}

/// The unit a layer is drawn in: the id of its holder, if it is inside one.
fn unit_members(doc: &Document, index: usize) -> usize {
    unit(doc, index).len()
}

pub fn holder_of(doc: &Document, id: u64) -> Option<u64> {
    let mut index = doc.layers.iter().position(|l| l.id == id)?;
    while inside(doc, index) {
        index -= 1;
    }
    Some(doc.layers[index].id)
}

/// What a layer looks like, without its name, lock, or position.
fn look(layer: &Layer) -> String {
    let mut layer = layer.clone();
    layer.transform.x = 0.;
    layer.transform.y = 0.;
    layer.name.clear();
    layer.locked = false;
    // Paint strokes and mask pixels go by their content hashes: writing
    // them out on every refresh would cost more than the render.
    let mut hashes = String::new();
    if let prism_core::LayerKind::Paint { program } = &mut layer.kind {
        hashes += &format!("|paint:{:?}", program.identity());
        if let Ok(empty) = prism_core::BrushProgram::new(program.width, program.height) {
            *program = empty;
        }
    }
    if let Some(mask) = &mut layer.pixel_mask {
        hashes += &format!("|mask:{:?}", mask.content_hash);
        mask.alpha = std::sync::Arc::from([]);
    }
    serde_json::to_string(&layer).unwrap_or_default() + &hashes
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
        let (Some(canvas), Ok(_)) = (&mut self.canvas, &self.store) else {
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
        // A move inside a unit changes how the unit looks, so it renders
        // with the layer where the drag has it.
        let moving = canvas.drag.and_then(|drag| {
            let unit = holder_of(&canvas.doc, drag.id)?;
            let index = canvas.doc.layers.iter().position(|l| l.id == unit)?;
            let members = unit_members(&canvas.doc, index);
            (members > 1 && drag.corner.is_none() && !drag.rotate).then_some(drag.id)
        });
        let moved = moving.and_then(|id| {
            let (_, dx, dy) = self.drag_delta()?;
            let mut layer = self.canvas.as_ref()?.doc.layer(id).ok()?.clone();
            layer.transform.x += dx;
            layer.transform.y += dy;
            Some(layer)
        });
        let (Some(canvas), Ok(store)) = (&mut self.canvas, &self.store) else {
            return;
        };
        let mut render_doc = canvas.render_doc();
        for changed in resizing.iter().chain(moved.iter()) {
            if let Some(slot) = render_doc.layers.iter_mut().find(|l| l.id == changed.id) {
                *slot = changed.clone();
            }
        }
        let cache = &mut canvas.cache;
        cache
            .images
            .retain(|id, _| render_doc.layers.iter().any(|l| l.id == *id));
        for (index, layer) in render_doc.layers.iter().enumerate() {
            if inside(&render_doc, index) {
                continue;
            }
            let members = unit(&render_doc, index);
            // A font being previewed in this unit is part of its look.
            let preview = canvas
                .font_preview
                .as_ref()
                .filter(|(previewed, _)| members.contains(previewed))
                .cloned();
            let font = preview.as_ref().map_or(String::new(), |(id, path)| {
                format!("|font:{id}:{}", path.display())
            });
            // The layers inside it, and where they sit relative to it.
            let inner: String = members[1..]
                .iter()
                .filter_map(|id| render_doc.layer(*id).ok())
                .map(|m| {
                    format!(
                        "|in:{}@{:.2},{:.2}",
                        look(m),
                        m.transform.x - layer.transform.x,
                        m.transform.y - layer.transform.y
                    )
                })
                .collect();
            let full = format!("{}{inner}{font}", key(layer, density));
            let current = cache.images.get(&layer.id).map(|i| i.key.as_str());
            // A stroke being drawn shows as a patch; its layer renders once
            // the stroke is applied, and the patch goes when that render is in.
            let patched = canvas
                .stroke_patch
                .as_ref()
                .filter(|p| members.contains(&p.layer))
                .map(|p| p.committed);
            if current == Some(full.as_str()) {
                if patched == Some(true)
                    && let Some(patch) = canvas.stroke_patch.take()
                {
                    window.drop_image(patch.image).ok();
                }
                continue;
            }
            if patched == Some(false) {
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
            // A layer being dragged (moved inside a unit, resized, or
            // rotated) always renders as a draft.
            let rapid = rapid || canvas.drag.is_some_and(|d| members.contains(&d.id));
            let render_density = if rapid {
                draft_density(canvas.bounds.get(&layer.id).copied(), &layer.style, density)
            } else {
                density
            };
            let wanted = if rapid {
                format!("{}{inner}{font}|draft", key(layer, render_density))
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
            let (root, doc, id, layer_id) =
                (store.root.clone(), render_doc.clone(), canvas.id, layer.id);
            let at = (layer.transform.x, layer.transform.y);
            let previewed = preview.as_ref().map(|(_, path)| path.clone());
            let task = cx.background_executor().spawn(async move {
                let doc = match preview {
                    Some((previewed, path)) => {
                        crate::font_browser::with_font(&doc, previewed, &path)?
                    }
                    None => doc,
                };
                render_alone(&root, &doc, &members, render_density)
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
                    if let Some(now) = canvas.shown().layers.iter().find(|l| l.id == layer_id)
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
