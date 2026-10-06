//! A real canvas open in the main area. Edits apply to a local copy of the
//! engine document at once and render in memory, newest first; the same
//! commands then save to the library in order.
use crate::{preview::to_render_image, workspace::Workspace};
use gpui::*;
use spectrum_assets::Service;
use spectrum_canvas::{Command, Document};
use spectrum_library::AssetId;
use std::{collections::HashMap, sync::Arc};

pub type Bounds2 = HashMap<u64, ([f32; 2], [f32; 2])>;

pub struct CanvasState {
    pub id: AssetId,
    /// The document with every edit applied, ahead of what is saved.
    pub doc: Document,
    pub image: Option<Arc<RenderImage>>,
    /// Render scale in device pixels per canvas unit: the canvas's scale on
    /// screen, so renders show pixel for pixel. Every part of the canvas
    /// renders at this one scale, on one pixel grid.
    pub density: f32,
    /// Canvas-space bounds of each layer, from the engine's geometry.
    pub bounds: Bounds2,
    pub selected: Option<u64>,
    pub drag: Option<LayerDrag>,
    pub guide_drag: Option<crate::guides::GuideDrag>,
    /// A drag or nudge landed; draw `split` until the render catches up.
    pub settling: bool,
    /// Each layer's own render, for canvases drawn layer by layer.
    pub cache: crate::layer_cache::LayerCache,
    /// The canvas rendered around one layer, so dragging it redraws at once.
    pub split: Option<crate::canvas_split::Split>,
    /// Image layers: edit the shared image rather than this placement.
    pub global: bool,
    /// A shared image being edited: its id, original file, and the edits
    /// not saved yet. Its layers render from the original with these edits
    /// until the save lands.
    pub shared_edit: Option<(
        spectrum_library::AssetId,
        std::path::PathBuf,
        spectrum_image::Adjustments,
    )>,
    /// A text layer shown in an installed font that is not applied yet.
    pub font_preview: Option<(u64, std::path::PathBuf)>,
    /// Where a dragged layer row would land in the list, top first.
    pub drop_slot: Option<usize>,
    /// The row a dragged layer would go inside instead, top first.
    pub drop_inside: Option<usize>,
    /// A text layer being edited on the canvas.
    pub editing: Option<u64>,
    pub tool: crate::tools::Tool,
    /// A layer being drawn with the current tool, from and to in canvas space.
    pub creating: Option<((f32, f32), (f32, f32))>,
    /// Points of a lasso or brush stroke being drawn, in canvas space.
    pub points: Vec<(f32, f32)>,
    /// How the selection being drawn combines with the current one.
    pub select_mode: spectrum_canvas::SelectionCombineMode,
    /// The selection's outline, keyed by the selection it was traced from.
    pub ants:
        std::cell::RefCell<Option<(u64, std::sync::Arc<[spectrum_canvas::SelectionOutlinePath]>)>>,
    /// Marching ants are animating.
    pub ants_running: bool,
    /// Anchors placed so far with the Pen, and where the pointer is.
    pub pen: Vec<crate::pen::PenPoint>,
    /// A Pen point was just placed and a drag pulls out its handles.
    pub pen_dragging: bool,
    pub pointer: Option<(f32, f32)>,
    /// A magic wand selection is being computed.
    pub wand_busy: bool,
    /// The guide under the pointer, which a drag would move.
    pub hover_guide: Option<spectrum_canvas::GuideOrientation>,
    /// The document with the brush stroke being drawn already applied, so
    /// its layer renders as the stroke goes; `doc` once it lands.
    pub live_doc: Option<Document>,
    /// A layer's transform mid-drag, for canvases drawn as one render
    /// (blend modes, layers inside others), so the render follows the drag.
    pub drag_preview: Option<(u64, spectrum_canvas::Transform)>,
    /// The part of a Paint layer under the stroke being drawn, rendered as
    /// it goes and drawn over the layer's image.
    pub stroke_patch: Option<StrokePatch>,
    /// The layer the Eraser started on, for layers other than Paint.
    pub erase_target: Option<u64>,
    /// The canvas's scale on screen, or `None` to fit it, and how far it is
    /// moved from the middle of the view, in screen pixels.
    pub zoom: Option<f32>,
    pub pan: (f32, f32),
    /// When the zoom last changed; it renders sharp once it settles.
    pub zoomed_at: Option<std::time::Instant>,
    /// A pan drag: where it started and the pan then.
    pub panning: Option<(gpui::Point<gpui::Pixels>, (f32, f32))>,
    /// Space is held: drags pan.
    pub space_held: bool,
    /// The flattened canvas at one pixel per unit, for the Eyedropper, and
    /// the look it was rendered from.
    pub pixels: Option<(u64, std::sync::Arc<image::RgbaImage>)>,
    pub pixels_busy: bool,
    /// Counts changes to the document; a reload only applies if none
    /// happened while it loaded.
    edits: u64,
    /// Counts reasons to render (edits, a new render size, a shared image's
    /// change); renders record the one they show.
    version: u64,
    rendered: u64,
    /// The saved document has arrived; until then `doc` is a placeholder.
    pub loaded: bool,
    rendering: bool,
    /// Commands applied locally and waiting to be saved.
    queue: Vec<Command>,
    saving: bool,
    /// When the document last changed; saves wait for a pause so one
    /// slider drag saves, and undoes, as one step.
    changed_at: Option<std::time::Instant>,
    save_waiting: bool,
    /// When a whole-canvas render was last asked for; renders asked for in
    /// quick succession are drafts, and a sharp one follows.
    render_asked: Option<std::time::Instant>,
    /// Reload from the library once saving finishes (after undo or redo).
    reload: bool,
}

impl CanvasState {
    /// Counts changes to how the canvas looks.
    pub fn look_version(&self) -> u64 {
        self.version
    }

    /// Something about how the canvas looks changed; renders catch up.
    pub fn bump_version(&mut self) {
        self.version += 1;
    }

    /// The document as drawn: with any stroke being drawn.
    pub fn shown(&self) -> &Document {
        self.live_doc.as_ref().unwrap_or(&self.doc)
    }

    /// Before `doc` replaces the document (undo, redo, a reload), moves each
    /// layer's image and bounds by how far the layer moved, so layers that
    /// only moved are drawn in place at once.
    pub fn follow_moves(&mut self, doc: &Document) {
        for layer in &doc.layers {
            let Some(old) = self.doc.layers.iter().find(|l| l.id == layer.id) else {
                continue;
            };
            let (t, o) = (layer.transform, old.transform);
            let only_moved =
                t.scale_x == o.scale_x && t.scale_y == o.scale_y && t.rotation == o.rotation;
            let (dx, dy) = (t.x - o.x, t.y - o.y);
            if !only_moved || (dx == 0. && dy == 0.) {
                continue;
            }
            if let Some(image) = self.cache.images.get_mut(&layer.id) {
                image.pixel = [
                    image.pixel[0] + dx * image.density,
                    image.pixel[1] + dy * image.density,
                ];
                let (min, max) = image.bounds;
                image.bounds = ([min[0] + dx, min[1] + dy], [max[0] + dx, max[1] + dy]);
            }
            if let Some((min, max)) = self.bounds.get_mut(&layer.id) {
                *min = [min[0] + dx, min[1] + dy];
                *max = [max[0] + dx, max[1] + dy];
            }
        }
    }

    /// The document to render: the saved one, with any shared image being
    /// edited drawn from its original file and the unsaved edits.
    pub fn render_doc(&self) -> Document {
        let mut doc = self.shown().clone();
        if let Some((id, transform)) = self.drag_preview
            && let Some(layer) = doc.layers.iter_mut().find(|l| l.id == id)
        {
            layer.transform = transform;
        }
        if let Some((asset, original, adjustments)) = &self.shared_edit {
            for layer in &mut doc.layers {
                let plain = layer.adjustments == spectrum_image::Adjustments::default();
                if layer.image_asset == Some(*asset) && plain {
                    layer.image_asset = None;
                    layer.kind = spectrum_canvas::LayerKind::Raster {
                        path: original.clone(),
                    };
                    layer.adjustments = adjustments.clone();
                }
            }
        }
        doc
    }
}

/// A Paint layer's pixels under a stroke, in canvas units.
pub struct StrokePatch {
    pub layer: u64,
    pub bounds: crate::canvas_split::LayerBounds,
    pub image: Arc<RenderImage>,
    /// The stroke has been applied; the patch goes once the layer re-renders.
    pub committed: bool,
}

#[derive(Clone, Copy)]
pub struct LayerDrag {
    pub id: u64,
    pub start: (f32, f32),
    pub now: (f32, f32),
    /// Resizing from this corner (right, bottom) instead of moving.
    pub corner: Option<(bool, bool)>,
    /// Rotating by the handle above the layer instead of moving.
    pub rotate: bool,
    /// Shift is held: rotation turns in fixed steps.
    pub constrain: bool,
}

pub struct Rendered {
    pub image: Arc<RenderImage>,
    pub bounds: Bounds2,
}

fn queue_key(command: &Command) -> Option<(String, u64)> {
    let value = serde_json::to_value(command).ok()?;
    let id = value.get("id")?.as_u64()?;
    Some((value.get("command")?.as_str()?.to_string(), id))
}

/// Resolves library images and renders the document with its layer bounds.
/// Renders a document whose library images are already resolved.
pub fn render_resolved(resolved: &Document, density: f32) -> anyhow::Result<Arc<RenderImage>> {
    let sources = spectrum_canvas::prepare_export_raster_sources(
        resolved,
        &spectrum_canvas::default_raster_backing_cache_root()?,
    )?;
    let image = spectrum_canvas::render_document_scaled_with_sources(resolved, density, &sources)?;
    Ok(to_render_image(image).0)
}

pub fn render_at(root: &std::path::Path, doc: &Document, density: f32) -> anyhow::Result<Rendered> {
    let mut resolved = doc.clone();
    Service::open(root)?.resolve(&mut resolved)?;
    let bounds = resolved
        .layers
        .iter()
        .filter_map(|layer| {
            let geometry = spectrum_canvas::document_layer_geometry(&resolved, layer).ok()?;
            Some((layer.id, (geometry.min, geometry.max)))
        })
        .collect();
    let sources = spectrum_canvas::prepare_export_raster_sources(
        &resolved,
        &spectrum_canvas::default_raster_backing_cache_root()?,
    )?;
    let image = spectrum_canvas::render_document_scaled_with_sources(&resolved, density, &sources)?;
    let image = to_render_image(image).0;
    Ok(Rendered { image, bounds })
}

impl Workspace {
    /// Opens a canvas asset: loads and renders it in the background.
    pub fn load_canvas(&mut self, id: AssetId, window: &mut Window, cx: &mut Context<Self>) {
        self.close_canvas(window);
        self.canvas = Some(CanvasState {
            id,
            doc: Document::new("", 1, 1),
            image: None,
            density: 1.,
            bounds: HashMap::new(),
            selected: None,
            drag: None,
            guide_drag: None,
            settling: false,
            split: None,
            cache: Default::default(),
            global: false,
            shared_edit: None,
            drop_slot: None,
            drop_inside: None,
            font_preview: None,
            editing: None,
            tool: Default::default(),
            creating: None,
            points: Vec::new(),
            select_mode: Default::default(),
            ants: Default::default(),
            ants_running: false,
            pen: Vec::new(),
            pen_dragging: false,
            pointer: None,
            wand_busy: false,
            hover_guide: None,
            live_doc: None,
            drag_preview: None,
            stroke_patch: None,
            erase_target: None,
            zoom: None,
            pan: (0., 0.),
            zoomed_at: None,
            panning: None,
            space_held: false,
            pixels: None,
            pixels_busy: false,
            edits: 0,
            version: 0,
            rendered: 0,
            loaded: false,
            rendering: false,
            queue: Vec::new(),
            saving: false,
            changed_at: None,
            save_waiting: false,
            render_asked: None,
            reload: true,
        });
        self.reload_canvas(window, cx);
    }

    /// Frees the open canvas's images.
    pub fn close_canvas(&mut self, window: &mut Window) {
        if let Some(mut old) = self.canvas.take() {
            Self::drop_layers(&mut old.cache, window);
            let split = old
                .split
                .into_iter()
                .flat_map(crate::canvas_split::Split::images);
            for image in old.image.into_iter().chain(split) {
                window.drop_image(image).ok();
            }
        }
    }

    /// Replaces the local document with the saved one when nothing is waiting to save.
    fn reload_canvas(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(canvas), Ok(store)) = (&mut self.canvas, &self.store) else {
            return;
        };
        if canvas.saving || !canvas.queue.is_empty() {
            canvas.reload = true;
            return;
        }
        canvas.reload = false;
        let (root, id, edits) = (store.root.clone(), canvas.id, canvas.edits);
        let task = cx
            .background_executor()
            .spawn(async move { Service::open(&root)?.saved_canvas(id) });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            this.update_in(cx, |this, window, cx| {
                let Some(canvas) = this.canvas.as_mut().filter(|c| c.id == id) else {
                    return;
                };
                match result {
                    // Newer local edits win; they are on their way to disk.
                    Ok(doc) if canvas.edits == edits => {
                        if canvas
                            .selected
                            .is_some_and(|s| !doc.layers.iter().any(|l| l.id == s))
                        {
                            canvas.selected = None;
                        }
                        canvas.follow_moves(&doc);
                        canvas.doc = doc;
                        canvas.edits += 1;
                        canvas.version += 1;
                        canvas.loaded = true;
                        if canvas.doc.selection.is_some() {
                            this.animate_ants(window, cx);
                        }
                        this.render_canvas(window, cx);
                        this.refresh_layers(window, cx);
                        this.sync_layer_controls(window, cx);
                    }
                    Ok(_) => {}
                    Err(error) => this.notify_error(error, window, cx),
                }
            })
            .ok();
        })
        .detach();
    }

    /// Renders again after something outside the document changed, such as
    /// a shared image's edits.
    pub fn rerender_canvas(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(canvas) = &mut self.canvas {
            canvas.version += 1;
        }
        self.split_edit(&[Command::Undo], window);
        self.invalidate_layers();
        self.render_canvas(window, cx);
        self.refresh_layers(window, cx);
    }

    /// Matches the render size to the canvas's size on screen, rendering
    /// again when the window or canvas size changes it.
    pub fn fit_canvas_resolution(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (_, scale) = self.canvas_rect();
        let fit = self.fit_scale() * window.scale_factor();
        let Some(canvas) = &mut self.canvas else {
            return;
        };
        // The placeholder before loading has no real size to fit.
        if !canvas.loaded
            || canvas.drag.is_some()
            || self.image_bounds.borrow().size.width <= px(1.)
        {
            return;
        }
        // Not while the zoom is still changing; layers stretch meanwhile.
        if canvas
            .zoomed_at
            .is_some_and(|at| at.elapsed() < crate::zoom::SETTLE)
        {
            return;
        }
        let long = canvas.doc.width.max(canvas.doc.height).max(1) as f32;
        // Zoomed in far, renders stop growing past four device pixels per
        // canvas pixel (or the fit, if larger) and are shown magnified.
        let wanted = (scale * window.scale_factor())
            .min(fit.max(4.))
            .clamp(0.05, 8192. / long);
        if (wanted - canvas.density).abs() <= canvas.density * 0.002 {
            return;
        }
        canvas.density = wanted;
        self.rerender_canvas(window, cx);
    }

    /// Renders the local document unless a render is running; the running
    /// one starts another when it finishes if edits arrived meanwhile.
    pub fn render_canvas(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(canvas), Ok(store)) = (&mut self.canvas, &self.store) else {
            return;
        };
        if canvas.rendering || canvas.rendered == canvas.version {
            return;
        }
        // Stackable canvases are drawn from layer images; no full render.
        if canvas.loaded && crate::layer_cache::stackable(&canvas.doc) {
            canvas.rendered = canvas.version;
            canvas.settling = false;
            return;
        }
        canvas.rendering = true;
        const RAPID: std::time::Duration = std::time::Duration::from_millis(150);
        let rapid = canvas.render_asked.is_some_and(|at| at.elapsed() < RAPID);
        canvas.render_asked = Some(std::time::Instant::now());
        let density = if rapid {
            (canvas.density * 0.5).max(0.5)
        } else {
            canvas.density
        };
        if rapid {
            // Render sharp once the changes stop.
            cx.spawn_in(window, async move |this, cx| {
                cx.background_executor()
                    .timer(RAPID + std::time::Duration::from_millis(30))
                    .await;
                this.update_in(cx, |this, window, cx| {
                    if let Some(canvas) = &mut this.canvas
                        && canvas.render_asked.is_some_and(|at| at.elapsed() >= RAPID)
                    {
                        canvas.version += 1;
                        this.render_canvas(window, cx);
                    }
                })
                .ok();
            })
            .detach();
        }
        let (root, id, version, doc) = (
            store.root.clone(),
            canvas.id,
            canvas.version,
            canvas.render_doc(),
        );
        let task = cx
            .background_executor()
            .spawn(async move { render_at(&root, &doc, density) });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            this.update_in(cx, |this, window, cx| {
                let Some(canvas) = this.canvas.as_mut().filter(|c| c.id == id) else {
                    if let Ok(done) = result {
                        window.drop_image(done.image).ok();
                    }
                    return;
                };
                canvas.rendering = false;
                canvas.rendered = version;
                match result {
                    Ok(done) => {
                        if let Some(old) = canvas.image.replace(done.image) {
                            window.drop_image(old).ok();
                        }
                        // Older renders keep the bounds later edits moved.
                        if canvas.rendered == canvas.version && canvas.drag.is_none() {
                            canvas.bounds = done.bounds;
                            canvas.settling = false;
                        }
                    }
                    Err(error) => this.notify_error(error, window, cx),
                }
                this.render_canvas(window, cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Applies engine commands to the open canvas now, then saves them.
    pub fn canvas_commands(
        &mut self,
        commands: Vec<Command>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let edit = self.split_edit(&commands, window);
        let Some(canvas) = &mut self.canvas else {
            return;
        };
        let history = commands
            .iter()
            .any(|c| matches!(c, Command::Undo | Command::Redo));
        // A crop moves every layer with the canvas's origin; move what is on
        // screen with them now rather than when each layer re-renders.
        let crop = commands.iter().find_map(|command| match command {
            Command::CropCanvas { x, y, .. } => Some((*x, *y)),
            Command::CropToSelection => canvas.doc.selection.as_ref().map(|s| {
                let (x, y, ..) = s.bounds();
                (x, y)
            }),
            _ => None,
        });
        if !history && !commands.is_empty() {
            let mut local = spectrum_canvas::Workspace::new(canvas.doc.clone());
            if let Err(error) = local.execute_batch(commands.clone()) {
                return self.notify_error(error, window, cx);
            }
            let doc = local.document;
            // Select what was just added, so it is ready to style.
            let added = doc
                .layers
                .iter()
                .filter(|l| !canvas.doc.layers.iter().any(|o| o.id == l.id))
                .map(|l| l.id)
                .max();
            if added.is_some() {
                canvas.selected = added;
            }
            if canvas
                .selected
                .is_some_and(|s| !doc.layers.iter().any(|l| l.id == s))
            {
                canvas.selected = None;
            }
            canvas.doc = doc;
            canvas.edits += 1;
            canvas.version += 1;
            if let Some((x, y)) = crop {
                let (dx, dy) = (-(x as f32), -(y as f32));
                canvas.cache.shift(dx, dy);
                for (min, max) in canvas.bounds.values_mut() {
                    *min = [min[0] + dx, min[1] + dy];
                    *max = [max[0] + dx, max[1] + dy];
                }
            }
        }
        canvas.changed_at = Some(std::time::Instant::now());
        if history {
            canvas.reload = true;
        }
        // A newer command for the same layer and property replaces a queued one,
        // so slider drags save only their latest value.
        for command in commands {
            let key = queue_key(&command);
            if key.is_some() {
                canvas.queue.retain(|queued| queue_key(queued) != key);
            }
            canvas.queue.push(command);
        }
        self.follow_split(edit, window, cx);
        self.render_canvas(window, cx);
        self.refresh_layers(window, cx);
        self.save_canvas(window, cx);
        self.sync_layer_controls(window, cx);
        cx.notify();
    }

    /// Saves queued commands one batch at a time.
    fn save_canvas(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(canvas), Ok(store)) = (&mut self.canvas, &self.store) else {
            return;
        };
        if canvas.saving || canvas.queue.is_empty() {
            return;
        }
        const PAUSE: std::time::Duration = std::time::Duration::from_millis(300);
        let quiet = canvas.changed_at.map_or(PAUSE, |at| at.elapsed());
        // Undo and redo show once saved, so they never wait.
        let history = matches!(canvas.queue.last(), Some(Command::Undo | Command::Redo));
        if quiet < PAUSE && !history {
            if !canvas.save_waiting {
                canvas.save_waiting = true;
                let wait = PAUSE - quiet;
                cx.spawn_in(window, async move |this, cx| {
                    cx.background_executor().timer(wait).await;
                    this.update_in(cx, |this, window, cx| {
                        if let Some(canvas) = &mut this.canvas {
                            canvas.save_waiting = false;
                        }
                        this.save_canvas(window, cx);
                    })
                    .ok();
                })
                .detach();
            }
            return;
        }
        canvas.saving = true;
        let batch = std::mem::take(&mut canvas.queue);
        let (root, id) = (store.root.clone(), canvas.id);
        let task = cx
            .background_executor()
            .spawn(async move { Service::open(&root)?.edit_canvas(id, batch) });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            this.update_in(cx, |this, window, cx| {
                let Some(canvas) = this.canvas.as_mut().filter(|c| c.id == id) else {
                    return;
                };
                canvas.saving = false;
                if let Err(error) = result {
                    // Show what the library really holds.
                    canvas.reload = true;
                    canvas.queue.clear();
                    this.notify_error(error, window, cx);
                }
                if let Ok(store) = &mut this.store {
                    store.thumbs.remove(&id);
                }
                let reload = this.canvas.as_ref().is_some_and(|c| c.reload);
                this.save_canvas(window, cx);
                if reload {
                    this.reload_canvas(window, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// Places images as linked layers; their previews render off the main thread.
    pub fn place_images(&mut self, ids: Vec<AssetId>, window: &mut Window, cx: &mut Context<Self>) {
        let Ok(store) = &self.store else {
            return;
        };
        let root = store.root.clone();
        let task = cx.background_executor().spawn(async move {
            let service = Service::open(&root)?;
            ids.into_iter()
                .map(|id| {
                    let name = service.library.get(id)?.name;
                    let path = service.preview(id)?;
                    Ok(Command::AddLinkedImage {
                        path,
                        name,
                        asset: id,
                    })
                })
                .collect::<anyhow::Result<Vec<_>>>()
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            this.update_in(cx, |this, window, cx| match result {
                Ok(commands) => this.canvas_commands(commands, window, cx),
                Err(error) => this.notify_error(error, window, cx),
            })
            .ok();
        })
        .detach();
    }

    /// Runs one command on the selected layer, if any.
    pub fn on_selected(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        command: impl FnOnce(u64) -> Command,
    ) {
        if let Some(id) = self.canvas.as_ref().and_then(|c| c.selected) {
            self.canvas_commands(vec![command(id)], window, cx);
        }
    }

    pub fn selected_layer(&self) -> Option<&spectrum_canvas::Layer> {
        let canvas = self.canvas.as_ref()?;
        let id = canvas.selected?;
        canvas.doc.layers.iter().find(|l| l.id == id)
    }
}
