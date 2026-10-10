//! A canvas shows an image edited since its last full render from a quick
//! stand-in (`Service::resolve_for_display`); its full render is made here in
//! the background, and the canvas redraws from it when it is done.
use crate::workspace::Workspace;
use gpui::*;
use spectrum_assets::Service;
use spectrum_library::AssetId;

/// Work nobody is waiting on (thumbnails) runs on a quarter of the cores, so
/// it never slows what a person is doing.
pub fn background<R: Send>(work: impl FnOnce() -> R + Send) -> R {
    static POOL: std::sync::OnceLock<Option<rayon::ThreadPool>> = std::sync::OnceLock::new();
    let pool = POOL.get_or_init(|| {
        let cores = std::thread::available_parallelism().map_or(4, |cores| cores.get());
        rayon::ThreadPoolBuilder::new()
            .num_threads((cores / 4).max(1))
            .thread_name(|index| format!("background-{index}"))
            .build()
            .ok()
    });
    match pool {
        Some(pool) => pool.install(work),
        None => work(),
    }
}

impl Workspace {
    /// Makes the full renders of `images`, unless already underway, then
    /// renders the open canvas again from them.
    pub fn make_full_previews(
        &mut self,
        images: Vec<AssetId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (Some(canvas), Ok(store)) = (&mut self.canvas, &self.store) else {
            return;
        };
        for image in images {
            if !canvas.full_previews.insert(image) {
                continue;
            }
            let (root, id) = (store.root.clone(), canvas.id);
            let task = cx.background_executor().spawn(async move {
                let started = std::time::Instant::now();
                // Made only once the image shows larger than its stand-in,
                // so the person is waiting for it: every core.
                let made = Service::open(&root)?.preview(image);
                crate::perf::record("full_preview", started.elapsed());
                made
            });
            cx.spawn_in(window, async move |this, cx| {
                let made = task.await;
                this.update_in(cx, |this, window, cx| {
                    let Some(canvas) = this.canvas.as_mut().filter(|c| c.id == id) else {
                        return;
                    };
                    canvas.full_previews.remove(&image);
                    if let Err(error) = made {
                        return this.notify_error(error, window, cx);
                    }
                    // Only the layers showing this image draw again.
                    let units: Vec<u64> = canvas
                        .doc
                        .layers
                        .iter()
                        .filter(|layer| layer.image_asset == Some(image))
                        .filter_map(|layer| crate::layer_cache::holder_of(&canvas.doc, layer.id))
                        .collect();
                    for unit in units {
                        let cache = &mut canvas.cache;
                        for drawn in [cache.images.get_mut(&unit), cache.details.get_mut(&unit)]
                            .into_iter()
                            .flatten()
                        {
                            drawn.key.clear();
                        }
                    }
                    this.redraw_canvas(window, cx);
                    this.refresh_layers(window, cx);
                })
                .ok();
            })
            .detach();
        }
    }
}
