//! A real canvas open in the main area: its engine document, latest render,
//! and layer geometry. Commands go to the engine one batch at a time.
use crate::workspace::Workspace;
use gpui::*;
use prism_core::{Command, Document};
use spectrum::library::{Service, live};
use spectrum_library::AssetId;
use std::{collections::HashMap, path::PathBuf};

/// Long edge of canvas renders.
const RENDER: u32 = 2048;

pub struct CanvasState {
    pub id: AssetId,
    pub doc: Document,
    pub render: Option<PathBuf>,
    /// Canvas-space bounds of each layer, from the engine's geometry.
    pub bounds: HashMap<u64, ([f32; 2], [f32; 2])>,
    pub selected: Option<u64>,
    /// Commands waiting while a batch runs.
    pub queue: Vec<Command>,
    pub busy: bool,
    pub drag: Option<LayerDrag>,
    /// Image layers: edit the shared image rather than this placement.
    pub global: bool,
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
    doc: Document,
    render: PathBuf,
    bounds: HashMap<u64, ([f32; 2], [f32; 2])>,
}

fn queue_key(command: &Command) -> Option<(String, u64)> {
    let value = serde_json::to_value(command).ok()?;
    let id = value.get("id")?.as_u64()?;
    Some((value.get("command")?.as_str()?.to_string(), id))
}

/// Applies `commands`, then loads and renders the result.
fn run(root: &std::path::Path, id: AssetId, commands: Vec<Command>) -> anyhow::Result<Rendered> {
    let service = Service::open(root)?;
    let asset = service.library.get(id)?;
    let path = service.library.path(&asset)?;
    if !commands.is_empty() {
        live::canvas(&path, commands)?;
    }
    let doc = prism_core::Workspace::load_read_only(&path)?;
    let mut resolved = doc.clone();
    service.resolve(&mut resolved)?;
    let bounds = resolved
        .layers
        .iter()
        .filter_map(|layer| {
            let geometry = prism_core::layer_geometry(layer).ok()?;
            Some((layer.id, (geometry.min, geometry.max)))
        })
        .collect();
    let sources = prism_core::prepare_export_raster_sources(
        &resolved,
        &prism_core::default_raster_backing_cache_root()?,
    )?;
    let image = prism_core::render_document_with_sources(&resolved, Some(RENDER), &sources)?;
    let render = std::env::temp_dir().join(format!("spectrum-canvas-{}.png", AssetId::new_v4()));
    image.save(&render)?;
    Ok(Rendered {
        doc,
        render,
        bounds,
    })
}

impl Workspace {
    /// Opens a canvas asset: loads and renders it in the background.
    pub fn load_canvas(&mut self, id: AssetId, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(old) = self.canvas.take().and_then(|c| c.render) {
            std::fs::remove_file(old).ok();
        }
        self.canvas = Some(CanvasState {
            id,
            doc: Document::new("", 1, 1),
            render: None,
            bounds: HashMap::new(),
            selected: None,
            queue: Vec::new(),
            busy: false,
            drag: None,
            global: false,
        });
        self.canvas_commands(Vec::new(), window, cx);
    }

    /// Queues engine commands for the open canvas and runs them in order.
    pub fn canvas_commands(
        &mut self,
        commands: Vec<Command>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (Some(canvas), Ok(store)) = (&mut self.canvas, &self.store) else {
            return;
        };
        // A newer command for the same layer and property replaces a queued one,
        // so slider drags send only their latest value.
        for command in commands {
            let key = queue_key(&command);
            if key.is_some() {
                canvas.queue.retain(|queued| queue_key(queued) != key);
            }
            canvas.queue.push(command);
        }
        if canvas.busy {
            return;
        }
        canvas.busy = true;
        let batch = std::mem::take(&mut canvas.queue);
        let (root, id) = (store.root.clone(), canvas.id);
        let task = cx
            .background_executor()
            .spawn(async move { run(&root, id, batch) });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            this.update_in(cx, |this, window, cx| {
                let Some(canvas) = &mut this.canvas else {
                    return;
                };
                if canvas.id != id {
                    return;
                }
                canvas.busy = false;
                match result {
                    Ok(done) => {
                        if let Some(old) = canvas.render.replace(done.render) {
                            std::fs::remove_file(old).ok();
                        }
                        if canvas
                            .selected
                            .is_some_and(|s| !done.doc.layers.iter().any(|l| l.id == s))
                        {
                            canvas.selected = None;
                        }
                        // Select what was just added, so it is ready to style.
                        let known = !canvas.doc.layers.is_empty();
                        let added = done
                            .doc
                            .layers
                            .iter()
                            .filter(|l| !canvas.doc.layers.iter().any(|o| o.id == l.id))
                            .map(|l| l.id)
                            .max();
                        if known && added.is_some() {
                            canvas.selected = added;
                        }
                        canvas.doc = done.doc;
                        canvas.bounds = done.bounds;
                        if let Ok(store) = &mut this.store {
                            store.thumbs.remove(&id);
                        }
                    }
                    Err(error) => this.notify_error(error, window, cx),
                }
                let pending = this.canvas.as_ref().is_some_and(|c| !c.queue.is_empty());
                if pending {
                    this.canvas_commands(Vec::new(), window, cx);
                }
                this.sync_layer_controls(window, cx);
                cx.notify();
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
