//! The open image, rendered in memory from a decoded source kept for the
//! session, so edits show on every frame of a drag. Only the newest edit
//! renders, and saving waits until edits pause.
use crate::{
    histogram::Histogram,
    workspace::{Mode, Open, Workspace},
};
use gpui::*;
use image::DynamicImage;
use lumen_core::Adjustments;
use spectrum::library::Service;
use spectrum_library::AssetId;
use std::{collections::VecDeque, sync::Arc, time::Duration};

/// Long edge of the decoded source, enough to fill a Retina display.
const SOURCE: u32 = 2560;
/// Quiet time after the last edit before it is saved to the library.
const SAVE_AFTER: Duration = Duration::from_millis(400);

pub struct Preview {
    pub id: AssetId,
    source: Option<Arc<DynamicImage>>,
    /// The latest render, and the adjustments it shows.
    pub image: Option<Arc<RenderImage>>,
    shown: Option<Adjustments>,
    /// The image with no adjustments, for Compare.
    pub original: Option<Arc<RenderImage>>,
    pub histogram: Option<Arc<Histogram>>,
    pub failed: bool,
    rendering: bool,
}

struct Rendered {
    image: Arc<RenderImage>,
    histogram: Histogram,
}

/// Converts engine pixels to the BGRA frame GPUI uploads directly.
pub fn to_render_image(image: DynamicImage) -> (Arc<RenderImage>, image::RgbaImage) {
    let rgba = image.into_rgba8();
    let mut bgra = rgba.clone();
    for pixel in bgra.pixels_mut() {
        pixel.0.swap(0, 2);
    }
    let frame = image::Frame::new(bgra);
    (Arc::new(RenderImage::new(vec![frame])), rgba)
}

fn render(source: &DynamicImage, adjustments: Adjustments) -> Rendered {
    let output = lumen_core::engine::render_preview_source(source.clone(), adjustments);
    let (image, rgba) = to_render_image(output);
    Rendered {
        image,
        histogram: Histogram::from_rgba(&rgba),
    }
}

/// Decodes the image once: the working source and its unedited render.
fn load(
    root: &std::path::Path,
    id: AssetId,
) -> anyhow::Result<(Arc<DynamicImage>, Arc<RenderImage>)> {
    let photo = Service::open(root)?.image(id)?;
    let (source, original) = lumen_core::engine::render_settled_preview_with_source(
        &photo,
        Adjustments::default(),
        SOURCE,
    )?;
    Ok((Arc::new(source), to_render_image(original).0))
}

impl Workspace {
    /// What the main area shows: the whole frame while cropping.
    fn view_adjustments(&self) -> Adjustments {
        if self.mode == Mode::Crop {
            Adjustments {
                crop: None,
                ..self.adjust.clone()
            }
        } else {
            self.adjust.clone()
        }
    }

    /// Starts decoding the open image when it changes.
    pub fn ensure_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Open::Image(id), Ok(store)) = (self.open, &self.store) else {
            return;
        };
        if self.preview.as_ref().is_some_and(|p| p.id == id) {
            return;
        }
        if let Some(old) = self.preview.take() {
            for image in old.image.into_iter().chain(old.original) {
                window.drop_image(image).ok();
            }
        }
        self.preview = Some(Preview {
            id,
            source: None,
            image: None,
            shown: None,
            original: None,
            histogram: None,
            failed: false,
            rendering: false,
        });
        let root = store.root.clone();
        let task = cx
            .background_executor()
            .spawn(async move { load(&root, id) });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            this.update_in(cx, |this, window, cx| {
                let Some(preview) = this.preview.as_mut().filter(|p| p.id == id) else {
                    return;
                };
                match result {
                    Ok((source, original)) => {
                        preview.source = Some(source);
                        preview.original = Some(original);
                        this.render_preview(window, cx);
                    }
                    Err(_) => preview.failed = true,
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Renders the current adjustments unless a render is running; the
    /// running one picks up the newest adjustments when it finishes.
    pub fn render_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let wanted = self.view_adjustments();
        let Some(preview) = &mut self.preview else {
            return;
        };
        let Some(source) = preview.source.clone() else {
            return;
        };
        if preview.rendering || preview.shown.as_ref() == Some(&wanted) {
            return;
        }
        preview.rendering = true;
        preview.shown = Some(wanted.clone());
        let id = preview.id;
        let task = cx
            .background_executor()
            .spawn(async move { render(&source, wanted) });
        cx.spawn_in(window, async move |this, cx| {
            let rendered = task.await;
            this.update_in(cx, |this, window, cx| {
                let Some(preview) = this.preview.as_mut().filter(|p| p.id == id) else {
                    window.drop_image(rendered.image).ok();
                    return;
                };
                preview.rendering = false;
                if let Some(old) = preview.image.replace(rendered.image) {
                    window.drop_image(old).ok();
                }
                preview.histogram = Some(Arc::new(rendered.histogram));
                this.render_preview(window, cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Queues the open image's adjustments to be saved once edits pause.
    pub fn schedule_save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.open_image() else {
            return;
        };
        let edits = &mut self.edits;
        if edits
            .pending
            .as_ref()
            .is_some_and(|(other, _)| *other != id)
        {
            edits.checkpoint();
        }
        edits.pending = Some((id, self.adjust.clone()));
        edits.generation += 1;
        let generation = edits.generation;
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(SAVE_AFTER).await;
            this.update_in(cx, |this, window, cx| {
                if this.edits.generation == generation {
                    this.save_now(window, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// Queues any waiting adjustments now, before navigation or history steps.
    pub fn save_now(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.edits.checkpoint();
        self.run_durable(window, cx);
    }

    /// Runs queued saves and history steps in order, one at a time, off the
    /// main thread. The view never waits for them.
    fn run_durable(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Ok(store), false) = (&self.store, self.edits.busy) else {
            return;
        };
        let Some(op) = self.edits.queue.pop_front() else {
            return;
        };
        self.edits.busy = true;
        let root = store.root.clone();
        let task = cx.background_executor().spawn({
            let op = op.clone();
            async move {
                let service = Service::open(&root)?;
                match op {
                    Durable::Save(id, adjustments) => service.set_adjustments(id, *adjustments),
                    Durable::Step(id, forward, _) => service.step_history(id, forward),
                }
            }
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            this.update_in(cx, |this, window, cx| {
                this.edits.busy = false;
                let id = match op {
                    Durable::Save(id, _) | Durable::Step(id, _, _) => id,
                };
                if let Ok(store) = &mut this.store {
                    store.thumbs.remove(&id);
                }
                match result {
                    // A step past this session's edits: show what the library holds.
                    Ok(()) if matches!(op, Durable::Step(_, _, true)) => {
                        if this.open_image() == Some(id) {
                            this.refresh_open_image(id, window, cx);
                        }
                    }
                    Ok(()) => {}
                    Err(error) => this.notify_error(error, window, cx),
                }
                this.run_durable(window, cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Command+Z and Command+Shift+Z on the open image: this session's edits
    /// step back and forth at once; the library catches up behind.
    pub fn step_image_history(
        &mut self,
        forward: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(id) = self.open_image() else {
            return;
        };
        let edits = &mut self.edits;
        edits.checkpoint();
        let shown = if forward {
            edits
                .future
                .pop()
                .inspect(|next| edits.history.push(next.clone()))
        } else if edits.history.len() > 1 {
            let current = edits.history.pop();
            edits.future.extend(current);
            edits.history.last().cloned()
        } else {
            None
        };
        // Redo does not survive between engine sessions, so a redone state is
        // saved again; stepping back works across sessions.
        edits.queue.push_back(match &shown {
            Some(next) if forward => Durable::Save(id, Box::new(next.clone())),
            _ => Durable::Step(id, forward, shown.is_none()),
        });
        if let Some(adjustments) = shown {
            self.adjust = adjustments;
            self.sync_color_sliders(window, cx);
            self.render_preview(window, cx);
        }
        self.run_durable(window, cx);
    }
}

/// Saving and history for the open image, kept in step with the library.
#[derive(Default)]
pub struct ImageEdits {
    /// Adjustments waiting for edits to pause.
    pending: Option<(AssetId, Adjustments)>,
    generation: u64,
    queue: VecDeque<Durable>,
    busy: bool,
    /// Adjustments after each save this session, oldest first; each entry
    /// matches one step of the library's history.
    history: Vec<Adjustments>,
    future: Vec<Adjustments>,
}

#[derive(Clone)]
enum Durable {
    Save(AssetId, Box<Adjustments>),
    /// Step back or forward; the flag reloads the image afterward.
    Step(AssetId, bool, bool),
}

impl ImageEdits {
    /// Starts history at the adjustments just loaded for an image.
    pub fn reset(&mut self, adjustments: Adjustments) {
        self.history = vec![adjustments];
        self.future.clear();
    }

    /// Turns waiting adjustments into a queued save and a history entry.
    fn checkpoint(&mut self) {
        let Some((id, adjustments)) = self.pending.take() else {
            return;
        };
        if self.history.last() == Some(&adjustments) {
            return;
        }
        self.history.push(adjustments.clone());
        self.future.clear();
        self.queue
            .push_back(Durable::Save(id, Box::new(adjustments)));
    }
}
