//! The canvas rendered in three parts around one layer: the layers below it,
//! the layer alone, and the layers above. Dragging the layer redraws from
//! these parts every frame, and editing only that layer re-renders just the
//! layer, so both feel instant while the full render catches up.
use crate::{canvas_state::render_at, workspace::Workspace};
use gpui::*;
use spectrum_assets::Service;
use spectrum_canvas::{Command, Document};
use std::sync::Arc;

/// A layer's top-left and bottom-right corners in canvas space.
pub type LayerBounds = ([f32; 2], [f32; 2]);

pub struct Split {
    pub layer: u64,
    /// The layer's bounds when its part was rendered; drawing maps them to
    /// where the layer is now.
    pub base: LayerBounds,
    /// The layer render's top-left pixel on the canvas render's grid, and
    /// its size in canvas units for scaling while resizing.
    pub pixel: [f32; 2],
    pub extent: [f32; 2],
    /// The scale the parts were rendered at.
    pub density: f32,
    pub parts: [Option<Arc<RenderImage>>; 3],
    /// A render of the layer alone is running; `stale` asks for another.
    busy: bool,
    stale: bool,
}

impl Split {
    pub fn ready(&self) -> Option<[Arc<RenderImage>; 3]> {
        let [below, alone, above] = &self.parts;
        Some([below.clone()?, alone.clone()?, above.clone()?])
    }

    pub fn images(self) -> impl Iterator<Item = Arc<RenderImage>> {
        self.parts.into_iter().flatten()
    }
}

/// How a batch of commands relates to the split.
#[derive(Clone, Copy)]
pub enum SplitEdit {
    /// Nothing the split can show.
    None,
    /// Moves or resizes `layer`; its part is still right.
    Moves,
    /// Changes only this layer; the parts around it are still right.
    Restyles(u64),
}

/// Commands that change `layer` and nothing else.
fn only_changes(command: &Command, layer: u64) -> bool {
    use Command::*;
    match command {
        LinkImage { id, .. }
        | UpdateText { id, .. }
        | SetTextTypography { id, .. }
        | UpdateRectangle { id, .. }
        | UpdateEllipse { id, .. }
        | RenameLayer { id, .. }
        | SetVisibility { id, .. }
        | SetOpacity { id, .. }
        | SetBlendMode { id, .. }
        | SetTransform { id, .. }
        | SetRotation { id, .. }
        | AlignLayer { id, .. }
        | AdjustLayer { id, .. }
        | SetLayerAdjustments { id, .. }
        | ResetLayerAdjustments { id, .. }
        | SetShapeStroke { id, .. }
        | SetLayerStyle { id, .. }
        | SetShapeFill { id, .. } => *id == layer,
        _ => false,
    }
}

/// The layers below `layer` on the canvas background, or those above it on
/// a clear one.
fn around(doc: &Document, layer: u64, above: bool) -> Document {
    let mut doc = doc.clone();
    let index = doc.layers.iter().position(|l| l.id == layer).unwrap_or(0);
    if above {
        doc.background = [0, 0, 0, 0];
    }
    for (i, each) in doc.layers.iter_mut().enumerate() {
        if (i > index) != above || i == index {
            each.visible = false;
        }
    }
    doc
}

/// Renders `layer` alone on a clear document around its bounds, so none of
/// it is lost outside the canvas. It renders at the canvas's scale and starts
/// on a whole pixel of the canvas's own render, so its pixels line up exactly
/// with the full render's. With `clip`, only the part of it within `clip`
/// (in canvas units) renders, as for the part of a large layer on screen.
pub fn render_alone(
    root: &std::path::Path,
    doc: &Document,
    layers: &[u64],
    density: f32,
    clip: Option<LayerBounds>,
) -> anyhow::Result<LayerRender> {
    // The first layer, and any layers inside it, which show only within it.
    let layer = layers.first().copied().unwrap_or_default();
    let mut alone = doc.clone();
    alone.background = [0, 0, 0, 0];
    alone.layers.retain(|l| layers.contains(&l.id));
    let mut resolved = alone.clone();
    let pending = Service::open(root)?.resolve_for_display(&mut resolved, density)?;
    let first = resolved
        .layers
        .first()
        .ok_or_else(|| anyhow::anyhow!("layer {layer} is gone"))?;
    let geometry = spectrum_canvas::document_layer_geometry(&resolved, first)?;
    let (min, max) = (geometry.min, geometry.max);
    // Room for effects that reach past the layer, such as a drop shadow.
    let reach = spectrum_canvas::style_reach(&first.style);
    let (mut from, mut to) = (
        [min[0] - reach, min[1] - reach],
        [max[0] + reach, max[1] + reach],
    );
    if let Some((clip_min, clip_max)) = clip {
        from = [from[0].max(clip_min[0]), from[1].max(clip_min[1])];
        to = [to[0].min(clip_max[0]), to[1].min(clip_max[1])];
        if to[0] <= from[0] || to[1] <= from[1] {
            anyhow::bail!("layer {layer} is not within the area to render");
        }
    }
    let pixel = [(from[0] * density).floor(), (from[1] * density).floor()];
    let end = [(to[0] * density).ceil(), (to[1] * density).ceil()];
    let origin = [pixel[0] / density, pixel[1] / density];
    let (width, height) = (
        ((end[0] - pixel[0]) / density).ceil().max(1.),
        ((end[1] - pixel[1]) / density).ceil().max(1.),
    );
    let members = resolved.layers[1..]
        .iter()
        .filter_map(|member| {
            let geometry = spectrum_canvas::document_layer_geometry(&resolved, member).ok()?;
            Some((member.id, (geometry.min, geometry.max)))
        })
        .collect();
    // Render the resolved copy, so the library is read once.
    let mut alone = resolved;
    alone.width = width as u32;
    alone.height = height as u32;
    for each in &mut alone.layers {
        each.transform.x -= origin[0];
        each.transform.y -= origin[1];
    }
    Ok(LayerRender {
        image: crate::canvas_state::render_resolved(&alone, density)?,
        bounds: (min, max),
        pixel,
        extent: [width, height],
        pending,
        members,
    })
}

/// A layer rendered alone ([`render_alone`]).
pub struct LayerRender {
    pub image: Arc<RenderImage>,
    /// The layer's bounds, in canvas units.
    pub bounds: LayerBounds,
    /// The render's top-left pixel on the canvas render's grid, and its size
    /// in canvas units.
    pub pixel: [f32; 2],
    pub extent: [f32; 2],
    /// Images drawn from stand-ins (see `Service::resolve_for_display`).
    pub pending: Vec<spectrum_library::AssetId>,
    /// The bounds of the layers drawn inside it.
    pub members: Vec<(u64, LayerBounds)>,
}

impl Workspace {
    /// Renders the canvas around `layer`, replacing any other split.
    pub fn prepare_split(&mut self, layer: u64, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(canvas), Ok(store)) = (&mut self.canvas, &self.store) else {
            return;
        };
        // Stackable canvases draw every layer from its own image. The others
        // draw a moving layer apart over the rest, so it keeps up with the
        // pointer; a layer that blends with what is under it shows unblended
        // until the drop renders the canvas whole.
        if !canvas.loaded || crate::layer_cache::stackable(&canvas.doc) {
            return;
        }
        // Drawn apart at the scale the whole canvas renders at.
        let density = canvas.density.min(crate::layer_cache::whole_density(
            Some((
                [0., 0.],
                [canvas.doc.width as f32, canvas.doc.height as f32],
            )),
            &Default::default(),
            canvas.density,
        ));
        if canvas.split.as_ref().is_some_and(|s| s.layer == layer) {
            return;
        }
        let Some(base) = canvas.bounds.get(&layer).copied() else {
            return;
        };
        if let Some(old) = canvas.split.take() {
            for image in old.images() {
                crate::retire::drop_later(image);
            }
        }
        canvas.split = Some(Split {
            layer,
            base,
            pixel: [base.0[0] * density, base.0[1] * density],
            extent: [base.1[0] - base.0[0], base.1[1] - base.0[1]],
            density,
            parts: [None, None, None],
            busy: false,
            stale: false,
        });
        let id = canvas.id;
        for above in [false, true] {
            let doc = around(&canvas.doc, layer, above);
            let root = store.root.clone();
            let task = cx
                .background_executor()
                .spawn(async move { render_at(&root, &doc, density) });
            cx.spawn_in(window, async move |this, cx| {
                let result = task.await;
                this.update_in(cx, |this, _, cx| {
                    let Ok(done) = result else {
                        return;
                    };
                    let split = this
                        .canvas
                        .as_mut()
                        .filter(|c| c.id == id)
                        .and_then(|c| c.split.as_mut())
                        .filter(|s| s.layer == layer);
                    let Some(split) = split else {
                        crate::retire::drop_later(done.image);
                        return;
                    };
                    if let Some(old) = split.parts[if above { 2 } else { 0 }].replace(done.image) {
                        crate::retire::drop_later(old);
                    }
                    cx.notify();
                })
                .ok();
            })
            .detach();
        }
        self.render_split_layer(window, cx);
    }

    /// Re-renders the split's layer alone, newest edit first.
    fn render_split_layer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(canvas), Ok(store)) = (&mut self.canvas, &self.store) else {
            return;
        };
        let (id, doc) = (canvas.id, canvas.render_doc());
        let Some(split) = &mut canvas.split else {
            return;
        };
        let density = split.density;
        if split.busy {
            split.stale = true;
            return;
        }
        split.busy = true;
        split.stale = false;
        let (root, layer) = (store.root.clone(), split.layer);
        let task = cx
            .background_executor()
            .spawn(async move { render_alone(&root, &doc, &[layer], density, None) });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            this.update_in(cx, |this, window, cx| {
                let Some(canvas) = this.canvas.as_mut().filter(|c| c.id == id) else {
                    return;
                };
                let dragging = canvas.drag.is_some();
                let Some(split) = canvas.split.as_mut().filter(|s| s.layer == layer) else {
                    if let Ok(LayerRender { image, .. }) = result {
                        crate::retire::drop_later(image);
                    }
                    return;
                };
                split.busy = false;
                let stale = split.stale;
                if let Ok(LayerRender {
                    image,
                    bounds,
                    pixel,
                    extent,
                    pending,
                    ..
                }) = result
                {
                    this.make_full_previews(pending, window, cx);
                    let Some(canvas) = this.canvas.as_mut() else {
                        return;
                    };
                    let Some(split) = canvas.split.as_mut() else {
                        return;
                    };
                    if let Some(old) = split.parts[1].replace(image) {
                        crate::retire::drop_later(old);
                    }
                    split.base = bounds;
                    split.pixel = pixel;
                    split.extent = extent;
                    split.density = density;
                    if !dragging {
                        canvas.bounds.insert(layer, bounds);
                    }
                }
                if stale {
                    this.render_split_layer(window, cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Sorts `commands` against the split before they apply, dropping the
    /// split when they change more than its layer.
    pub fn split_edit(&mut self, commands: &[Command]) -> SplitEdit {
        let Some(canvas) = &mut self.canvas else {
            return SplitEdit::None;
        };
        let target = canvas.split.as_ref().map(|s| s.layer).or(canvas.selected);
        let edit = match target {
            _ if commands.is_empty() => SplitEdit::None,
            Some(layer)
                if commands
                    .iter()
                    .all(|c| matches!(c, Command::SetTransform { id, .. } if *id == layer)) =>
            {
                SplitEdit::Moves
            }
            Some(layer) if commands.iter().all(|c| only_changes(c, layer)) => {
                SplitEdit::Restyles(layer)
            }
            _ => SplitEdit::None,
        };
        let keep = match edit {
            SplitEdit::None => false,
            SplitEdit::Moves => true,
            SplitEdit::Restyles(layer) => canvas.split.as_ref().is_some_and(|s| s.layer == layer),
        };
        if !keep && let Some(split) = canvas.split.take() {
            for image in split.images() {
                crate::retire::drop_later(image);
            }
            canvas.settling = false;
        }
        edit
    }

    /// After a restyle applies, redraws its layer from the split at once.
    pub fn follow_split(&mut self, edit: SplitEdit, window: &mut Window, cx: &mut Context<Self>) {
        let SplitEdit::Restyles(layer) = edit else {
            return;
        };
        if let Some(canvas) = &mut self.canvas {
            canvas.settling = true;
        }
        if self
            .canvas
            .as_ref()
            .is_some_and(|c| c.split.as_ref().is_some_and(|s| s.layer == layer))
        {
            self.render_split_layer(window, cx);
        } else {
            self.prepare_split(layer, window, cx);
        }
    }
}
