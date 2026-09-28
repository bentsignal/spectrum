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
use std::{sync::Arc, time::Duration};

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
        if self
            .pending_save
            .as_ref()
            .is_some_and(|(other, _)| *other != id)
        {
            self.save_now(window, cx);
        }
        self.pending_save = Some((id, self.adjust.clone()));
        self.save_generation += 1;
        let generation = self.save_generation;
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(SAVE_AFTER).await;
            this.update_in(cx, |this, window, cx| {
                if this.save_generation == generation {
                    this.save_in_background(window, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// Saves queued adjustments off the main thread, one save at a time.
    fn save_in_background(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        let (Some((id, adjustments)), Ok(store)) = (self.pending_save.take(), &self.store) else {
            return;
        };
        self.saving = true;
        let (root, lock) = (store.root.clone(), self.save_lock.clone());
        let task = cx.background_executor().spawn(async move {
            let _order = lock.lock();
            Service::open(&root)?.set_adjustments(id, adjustments)
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            this.update_in(cx, |this, window, cx| {
                this.saving = false;
                if let Ok(store) = &mut this.store {
                    store.thumbs.remove(&id);
                }
                if let Err(error) = result {
                    this.notify_error(error, window, cx);
                }
                // Edits that arrived meanwhile and have already gone quiet.
                if this.pending_save.is_some() {
                    this.save_in_background(window, cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Saves queued adjustments now, after any save already running, so
    /// history steps and other readers see them.
    pub fn save_now(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some((id, adjustments)), Ok(store)) = (self.pending_save.take(), &mut self.store)
        else {
            return;
        };
        let lock = self.save_lock.clone();
        let result = {
            let _order = lock.lock();
            store.service.set_adjustments(id, adjustments)
        };
        store.thumbs.remove(&id);
        if let Err(error) = result {
            self.notify_error(error, window, cx);
        }
    }
}
