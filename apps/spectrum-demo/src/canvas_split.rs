//! The canvas rendered in three parts around one layer: the layers below it,
//! the layer alone, and the layers above. Dragging the layer redraws from
//! these parts every frame, and editing only that layer re-renders just the
//! layer, so both feel instant while the full render catches up.
use crate::{canvas_state::render_at, workspace::Workspace};
use gpui::*;
use prism_core::{Command, Document};
use spectrum::library::Service;
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
/// with the full render's. Returns the render, the layer's bounds, its
/// top-left pixel on the canvas render's grid, and its size in canvas units.
pub fn render_alone(
    root: &std::path::Path,
    doc: &Document,
    layer: u64,
    density: f32,
) -> anyhow::Result<(Arc<RenderImage>, LayerBounds, [[f32; 2]; 2])> {
    let mut alone = doc.clone();
    alone.background = [0, 0, 0, 0];
    alone.layers.retain(|l| l.id == layer);
    let mut resolved = alone.clone();
    Service::open(root)?.resolve(&mut resolved)?;
    let first = resolved
        .layers
        .first()
        .ok_or_else(|| anyhow::anyhow!("layer {layer} is gone"))?;
    let geometry = prism_core::document_layer_geometry(&resolved, first)?;
    let (min, max) = (geometry.min, geometry.max);
    // Room for effects that reach past the layer, such as a drop shadow.
    let reach = prism_core::style_reach(&first.style);
    let pixel = [
        ((min[0] - reach) * density).floor(),
        ((min[1] - reach) * density).floor(),
    ];
    let end = [
        ((max[0] + reach) * density).ceil(),
        ((max[1] + reach) * density).ceil(),
    ];
    let origin = [pixel[0] / density, pixel[1] / density];
    let (width, height) = (
        ((end[0] - pixel[0]) / density).ceil().max(1.),
        ((end[1] - pixel[1]) / density).ceil().max(1.),
    );
    alone.width = width as u32;
    alone.height = height as u32;
    for each in &mut alone.layers {
        each.transform.x -= origin[0];
        each.transform.y -= origin[1];
    }
    Ok((
        render_at(root, &alone, density)?.image,
        (min, max),
        [pixel, [width, height]],
    ))
}

impl Workspace {
    /// Renders the canvas around `layer`, replacing any other split.
    pub fn prepare_split(&mut self, layer: u64, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(canvas), Ok(store)) = (&mut self.canvas, &self.store) else {
            return;
        };
        // Stackable canvases draw every layer from its own image instead.
        if crate::layer_cache::stackable(&canvas.doc) {
            return;
        }
        if canvas.split.as_ref().is_some_and(|s| s.layer == layer) {
            return;
        }
        let Some(base) = canvas.bounds.get(&layer).copied() else {
            return;
        };
        if let Some(old) = canvas.split.take() {
            for image in old.images() {
                window.drop_image(image).ok();
            }
        }
        canvas.split = Some(Split {
            layer,
            base,
            pixel: [base.0[0] * canvas.density, base.0[1] * canvas.density],
            extent: [base.1[0] - base.0[0], base.1[1] - base.0[1]],
            density: canvas.density,
            parts: [None, None, None],
            busy: false,
            stale: false,
        });
        let id = canvas.id;
        for above in [false, true] {
            let (doc, density) = (around(&canvas.doc, layer, above), canvas.density);
            let root = store.root.clone();
            let task = cx
                .background_executor()
                .spawn(async move { render_at(&root, &doc, density) });
            cx.spawn_in(window, async move |this, cx| {
                let result = task.await;
                this.update_in(cx, |this, window, cx| {
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
                        window.drop_image(done.image).ok();
                        return;
                    };
                    if let Some(old) = split.parts[if above { 2 } else { 0 }].replace(done.image) {
                        window.drop_image(old).ok();
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
        let (id, doc, density) = (canvas.id, canvas.render_doc(), canvas.density);
        let Some(split) = &mut canvas.split else {
            return;
        };
        if split.busy {
            split.stale = true;
            return;
        }
        split.busy = true;
        split.stale = false;
        let (root, layer) = (store.root.clone(), split.layer);
        let task = cx
            .background_executor()
            .spawn(async move { render_alone(&root, &doc, layer, density) });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            this.update_in(cx, |this, window, cx| {
                let Some(canvas) = this.canvas.as_mut().filter(|c| c.id == id) else {
                    return;
                };
                let dragging = canvas.drag.is_some();
                let Some(split) = canvas.split.as_mut().filter(|s| s.layer == layer) else {
                    if let Ok((image, ..)) = result {
                        window.drop_image(image).ok();
                    }
                    return;
                };
                split.busy = false;
                let stale = split.stale;
                if let Ok((image, bounds, [pixel, extent])) = result {
                    if let Some(old) = split.parts[1].replace(image) {
                        window.drop_image(old).ok();
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
    pub fn split_edit(&mut self, commands: &[Command], window: &mut Window) -> SplitEdit {
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
                window.drop_image(image).ok();
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
