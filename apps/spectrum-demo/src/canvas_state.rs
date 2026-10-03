//! A real canvas open in the main area. Edits apply to a local copy of the
//! engine document at once and render in memory, newest first; the same
//! commands then save to the library in order.
use crate::{preview::to_render_image, workspace::Workspace};
use gpui::*;
use prism_core::{Command, Document};
use spectrum::library::{Service, live};
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
        lumen_core::Adjustments,
    )>,
    /// A text layer shown in an installed font that is not applied yet.
    pub font_preview: Option<(u64, std::path::PathBuf)>,
    /// Where a dragged layer row would land in the list, top first.
    pub drop_slot: Option<usize>,
    /// A text layer being edited on the canvas.
    pub editing: Option<u64>,
    pub tool: crate::tools::Tool,
    /// A layer being drawn with the current tool, from and to in canvas space.
    pub creating: Option<((f32, f32), (f32, f32))>,
    /// Points of a lasso or brush stroke being drawn, in canvas space.
    pub points: Vec<(f32, f32)>,
    /// How the selection being drawn combines with the current one.
    pub select_mode: prism_core::SelectionCombineMode,
    /// The selection's outline, keyed by the selection it was traced from.
    pub ants: std::cell::RefCell<Option<(u64, std::sync::Arc<[prism_core::SelectionOutlinePath]>)>>,
    /// Marching ants are animating.
    pub ants_running: bool,
    /// Anchors placed so far with the Pen, and where the pointer is.
    pub pen: Vec<(f32, f32)>,
    pub pointer: Option<(f32, f32)>,
    /// A magic wand selection is being computed.
    pub wand_busy: bool,
    /// The guide under the pointer, which a drag would move.
    pub hover_guide: Option<prism_core::GuideOrientation>,
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
    /// Reload from the library once saving finishes (after undo or redo).
    reload: bool,
}

impl CanvasState {
    /// The document to render: the saved one, with any shared image being
    /// edited drawn from its original file and the unsaved edits.
    pub fn render_doc(&self) -> Document {
        let mut doc = self.doc.clone();
        if let Some((asset, original, adjustments)) = &self.shared_edit {
            for layer in &mut doc.layers {
                let plain = layer.adjustments == lumen_core::Adjustments::default();
                if layer.image_asset == Some(*asset) && plain {
                    layer.image_asset = None;
                    layer.kind = prism_core::LayerKind::Raster {
                        path: original.clone(),
                        original_path: None,
                    };
                    layer.adjustments = adjustments.clone();
                }
            }
        }
        doc
    }
}

#[derive(Clone, Copy)]
pub struct LayerDrag {
    pub id: u64,
    pub start: (f32, f32),
    pub now: (f32, f32),
    /// Resizing from this corner (right, bottom) instead of moving.
    pub corner: Option<(bool, bool)>,
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

fn canvas_path(root: &std::path::Path, id: AssetId) -> anyhow::Result<std::path::PathBuf> {
    let service = Service::open(root)?;
    let asset = service.library.get(id)?;
    service.library.path(&asset)
}

/// Resolves library images and renders the document with its layer bounds.
pub fn render_at(root: &std::path::Path, doc: &Document, density: f32) -> anyhow::Result<Rendered> {
    let mut resolved = doc.clone();
    Service::open(root)?.resolve(&mut resolved)?;
    let bounds = resolved
        .layers
        .iter()
        .filter_map(|layer| {
            let geometry = prism_core::document_layer_geometry(&resolved, layer).ok()?;
            Some((layer.id, (geometry.min, geometry.max)))
        })
        .collect();
    let sources = prism_core::prepare_export_raster_sources(
        &resolved,
        &prism_core::default_raster_backing_cache_root()?,
    )?;
    let image = prism_core::render_document_scaled_with_sources(&resolved, density, &sources)?;
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
            font_preview: None,
            editing: None,
            tool: Default::default(),
            creating: None,
            points: Vec::new(),
            select_mode: Default::default(),
            ants: Default::default(),
            ants_running: false,
            pen: Vec::new(),
            pointer: None,
            wand_busy: false,
            hover_guide: None,
            edits: 0,
            version: 0,
            rendered: 0,
            loaded: false,
            rendering: false,
            queue: Vec::new(),
            saving: false,
            changed_at: None,
            save_waiting: false,
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
            .spawn(async move { prism_core::Workspace::load_read_only(&canvas_path(&root, id)?) });
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
        let long = canvas.doc.width.max(canvas.doc.height).max(1) as f32;
        let wanted = (scale * window.scale_factor()).clamp(0.05, 8192. / long);
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
        let (root, id, version, doc, density) = (
            store.root.clone(),
            canvas.id,
            canvas.version,
            canvas.render_doc(),
            canvas.density,
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
        if !history && !commands.is_empty() {
            let mut local = prism_core::Workspace::new(canvas.doc.clone(), None);
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
            .spawn(async move { live::canvas(&canvas_path(&root, id)?, batch) });
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

    pub fn selected_layer(&self) -> Option<&prism_core::Layer> {
        let canvas = self.canvas.as_ref()?;
        let id = canvas.selected?;
        canvas.doc.layers.iter().find(|l| l.id == id)
    }
}
