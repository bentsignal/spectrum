//! The open image, rendered in memory from a decoded source kept for the
//! session, so edits show on every frame of a drag. Only the newest edit
//! renders, and saving waits until edits pause. A drag shows the largest
//! frames this computer renders within a 60 Hz frame for these edits, and
//! the full-size frame follows as soon as the drag pauses.
use crate::{
    histogram::Histogram,
    workspace::{Mode, Open, Workspace},
};
use gpui::*;
use image::DynamicImage;
use rayon::prelude::*;
use spectrum_assets::Service;
use spectrum_image::{Adjustments, render::StagedRender};
use spectrum_library::AssetId;
use std::{
    collections::VecDeque,
    sync::Arc,
    time::{Duration, Instant},
};

/// Long edge of the decoded source, enough to fill a Retina display.
const SOURCE: u32 = 2560;
/// Long edges of the sources a drag may use, largest first.
const LEVELS: [u32; 3] = [SOURCE, SOURCE / 2, SOURCE / 3];
/// The longest a frame may take during a drag: one 60 Hz frame.
const FRAME: Duration = Duration::from_millis(16);
/// Edits closer together than this are one drag.
const DRAG_GAP: Duration = Duration::from_millis(250);
/// Quiet time after a drag before its full-size frame renders.
const SHARP_AFTER: Duration = Duration::from_millis(60);
/// Quiet time after the last edit before it is saved to the library.
const SAVE_AFTER: Duration = Duration::from_millis(400);

/// The open image's decoded pixels at each of [`LEVELS`].
struct Sources {
    levels: Vec<StagedRender>,
}

pub struct Preview {
    pub id: AssetId,
    sources: Option<Arc<Sources>>,
    /// The latest render, and the adjustments it shows.
    pub image: Option<Arc<RenderImage>>,
    shown: Option<Adjustments>,
    /// Whether the latest render is full size.
    sharp: bool,
    /// How long the latest frame at each level took.
    costs: [Option<Duration>; LEVELS.len()],
    /// The newest adjustments asked for, when, and the gap before them.
    requested: Option<Adjustments>,
    last_edit: Option<Instant>,
    edit_gap: Duration,
    /// The image with no adjustments, for Compare.
    pub original: Option<Arc<RenderImage>>,
    pub histogram: Option<Arc<Histogram>>,
    pub failed: bool,
    rendering: bool,
    /// When the oldest edit not yet on screen was made.
    unseen_since: Option<Instant>,
}

struct Rendered {
    image: Arc<RenderImage>,
    histogram: Histogram,
}

/// Converts engine pixels to the BGRA frame GPUI uploads directly.
pub fn to_render_image(image: DynamicImage) -> (Arc<RenderImage>, image::RgbaImage) {
    let rgba = image.into_rgba8();
    (frame(rgba.clone()), rgba)
}

/// Swaps RGBA to BGRA in place, on every core, and wraps it as a frame.
fn frame(mut pixels: image::RgbaImage) -> Arc<RenderImage> {
    pixels.par_chunks_mut(4 * 4096).for_each(|chunk| {
        for pixel in chunk.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }
    });
    Arc::new(RenderImage::new(vec![image::Frame::new(pixels)]))
}

fn render(source: &StagedRender, adjustments: Adjustments) -> Rendered {
    let pixels = source.render(adjustments);
    let histogram = Histogram::from_rgba(&pixels);
    Rendered {
        image: frame(pixels),
        histogram,
    }
}

/// The largest level whose frames render within [`FRAME`], from what each
/// level last took or, unmeasured, from a measured level scaled by its area.
fn drag_level(costs: &[Option<Duration>; LEVELS.len()]) -> usize {
    let estimate = |level: usize| {
        costs[level].or_else(|| {
            costs.iter().enumerate().find_map(|(measured, cost)| {
                let scale = LEVELS[level] as f32 / LEVELS[measured] as f32;
                cost.map(|cost| cost.mul_f32(scale * scale))
            })
        })
    };
    (0..LEVELS.len())
        .find(|level| estimate(*level).is_none_or(|cost| cost <= FRAME))
        .unwrap_or(LEVELS.len() - 1)
}

/// Decodes the image once into its working sources; unedited, the full-size
/// source is also the original Compare shows.
fn load(root: &std::path::Path, id: AssetId) -> anyhow::Result<(Sources, Arc<RenderImage>)> {
    let photo = Service::open(root)?.image(id)?;
    let source = spectrum_image::engine::decode_photo(&photo, Some(SOURCE))?;
    let smaller: Vec<_> = LEVELS[1..]
        .par_iter()
        .map(|size| StagedRender::new(spectrum_image::downscale(&source, *size)))
        .collect();
    let original = frame(source.to_rgba8());
    let levels = std::iter::once(StagedRender::new(source))
        .chain(smaller)
        .collect();
    Ok((Sources { levels }, original))
}

impl Workspace {
    /// Whether the open image shows its current edits and has saved them.
    pub fn image_idle(&self) -> bool {
        let wanted = self.view_adjustments();
        self.image.preview.as_ref().is_some_and(|preview| {
            preview.image.is_some()
                && preview.sharp
                && !preview.rendering
                && preview.shown.as_ref() == Some(&wanted)
        }) && self.image.edits.idle()
    }

    /// What the main area shows: the whole frame while cropping.
    fn view_adjustments(&self) -> Adjustments {
        if self.mode == Mode::Crop {
            Adjustments {
                crop: None,
                ..self.image.adjust.clone()
            }
        } else {
            self.image.adjust.clone()
        }
    }

    /// Starts decoding the open image when it changes.
    pub fn ensure_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Open::Image(id), Ok(store)) = (self.open, &self.store) else {
            return;
        };
        if self.image.preview.as_ref().is_some_and(|p| p.id == id) {
            return;
        }
        if let Some(old) = self.image.preview.take() {
            for image in old.image.into_iter().chain(old.original) {
                window.drop_image(image).ok();
            }
        }
        self.image.preview = Some(Preview {
            id,
            sources: None,
            image: None,
            shown: None,
            sharp: false,
            costs: [None; LEVELS.len()],
            requested: None,
            last_edit: None,
            edit_gap: Duration::MAX,
            original: None,
            histogram: None,
            failed: false,
            rendering: false,
            unseen_since: None,
        });
        let root = store.root.clone();
        let task = cx
            .background_executor()
            .spawn(async move { load(&root, id) });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            this.update_in(cx, |this, window, cx| {
                let Some(preview) = this.image.preview.as_mut().filter(|p| p.id == id) else {
                    return;
                };
                match result {
                    Ok((sources, original)) => {
                        preview.sources = Some(Arc::new(sources));
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
    /// running one picks up the newest adjustments when it finishes. During
    /// a drag, slow images render half size, then full size once it pauses.
    pub fn render_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let wanted = self.view_adjustments();
        let Some(preview) = &mut self.image.preview else {
            return;
        };
        let Some(sources) = preview.sources.clone() else {
            return;
        };
        let now = Instant::now();
        if preview.requested.as_ref() != Some(&wanted) {
            preview.edit_gap = preview.last_edit.map_or(Duration::MAX, |last| now - last);
            preview.last_edit = Some(now);
            preview.requested = Some(wanted.clone());
        }
        let current = preview.shown.as_ref() == Some(&wanted);
        if !current {
            preview.unseen_since.get_or_insert(now);
        }
        if preview.rendering || (current && preview.sharp) {
            return;
        }
        let dragging = preview.edit_gap < DRAG_GAP;
        let level = if !current && dragging {
            drag_level(&preview.costs)
        } else {
            0
        };
        let draft = level > 0;
        // The full-size frame after a drag waits for the drag to pause.
        let quiet = preview.last_edit.map_or(SHARP_AFTER, |last| now - last);
        if current && quiet < SHARP_AFTER {
            cx.spawn_in(window, async move |this, cx| {
                cx.background_executor().timer(SHARP_AFTER - quiet).await;
                this.update_in(cx, |this, window, cx| this.render_preview(window, cx))
                    .ok();
            })
            .detach();
            return;
        }
        preview.rendering = true;
        preview.shown = Some(wanted.clone());
        let covers = preview.unseen_since.take();
        let edited = preview.last_edit;
        let id = preview.id;
        let task = cx.background_executor().spawn(async move {
            let started = Instant::now();
            let rendered = render(&sources.levels[level], wanted);
            (rendered, started.elapsed())
        });
        cx.spawn_in(window, async move |this, cx| {
            let (rendered, cost) = task.await;
            this.update_in(cx, |this, window, cx| {
                let Some(preview) = this.image.preview.as_mut().filter(|p| p.id == id) else {
                    window.drop_image(rendered.image).ok();
                    return;
                };
                preview.rendering = false;
                preview.sharp = !draft;
                // A fresh full-size cost re-estimates the smaller levels, as
                // edits may have grown lighter or heavier since they ran.
                if level == 0 {
                    preview.costs = [None; LEVELS.len()];
                }
                preview.costs[level] = Some(cost);
                crate::perf::record(
                    if draft {
                        "image_draft_render"
                    } else {
                        "image_preview_render"
                    },
                    cost,
                );
                if let Some(old) = preview.image.replace(rendered.image) {
                    window.drop_image(old).ok();
                }
                preview.histogram = Some(Arc::new(rendered.histogram));
                // The oldest edit this frame shows was made this long ago.
                if let Some(since) = covers {
                    crate::perf::record("image_edit_to_screen", since.elapsed());
                }
                // How long after the last edit the image showed it sharp.
                if !draft
                    && preview.requested == preview.shown
                    && let Some(edited) = edited
                {
                    crate::perf::record("image_sharp", edited.elapsed());
                }
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
        let edits = &mut self.image.edits;
        if edits
            .pending
            .as_ref()
            .is_some_and(|(other, _)| *other != id)
        {
            edits.checkpoint();
        }
        edits.pending = Some((id, self.image.adjust.clone()));
        edits.generation += 1;
        let generation = edits.generation;
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(SAVE_AFTER).await;
            this.update_in(cx, |this, window, cx| {
                if this.image.edits.generation == generation {
                    this.save_now(window, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// Queues any waiting adjustments now, before navigation or history steps.
    pub fn save_now(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.image.edits.checkpoint();
        self.run_durable(window, cx);
    }

    /// Runs queued saves and history steps in order, one at a time, off the
    /// main thread. The view never waits for them.
    fn run_durable(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Ok(store), false) = (&self.store, self.image.edits.busy) else {
            return;
        };
        let Some(op) = self.image.edits.queue.pop_front() else {
            return;
        };
        self.image.edits.busy = true;
        let root = store.root.clone();
        let id = match op {
            Durable::Save(id, _) | Durable::Step(id, _, _) => id,
        };
        // Edits continue from the revision on screen, never from somewhere
        // an agent has moved on to.
        let base = self.image.edits.revisions.get(&id).copied();
        let task = cx.background_executor().spawn({
            let op = op.clone();
            async move {
                let command = match op {
                    Durable::Save(_, adjustments) => spectrum_image::Command::SetAdjustments {
                        adjustments: *adjustments,
                    },
                    Durable::Step(_, true, _) => spectrum_image::Command::Redo,
                    Durable::Step(_, false, _) => spectrum_image::Command::Undo,
                };
                let started = std::time::Instant::now();
                let service = Service::open(&root)?;
                let result =
                    service
                        .edit_image_from(id, base, vec![command])
                        .map(|(_, revision)| {
                            (revision, crate::following::document_stamp(&service, id))
                        });
                crate::perf::record("image_save", started.elapsed());
                result
            }
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            this.update_in(cx, |this, window, cx| {
                this.image.edits.busy = false;
                if let Ok((revision, stamp)) = &result {
                    this.image.edits.revisions.insert(id, *revision);
                    this.saved_stamp(id, *stamp);
                }
                if let Ok(store) = &mut this.store {
                    store.thumbs.remove(&id);
                }
                match result {
                    // A step past this session's edits: show what the library holds.
                    Ok(_) if matches!(op, Durable::Step(_, _, true)) => {
                        if this.open_image() == Some(id) {
                            this.refresh_open_image(id, window, cx);
                        }
                    }
                    Ok(_) => {}
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
        let edits = &mut self.image.edits;
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
        // The library's history steps the same way; with nothing local to
        // show, the step reloads what the library holds.
        edits
            .queue
            .push_back(Durable::Step(id, forward, shown.is_none()));
        if let Some(adjustments) = shown {
            self.image.adjust = adjustments;
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
    /// The revision each image's edits continue from: the one on screen.
    pub revisions: std::collections::HashMap<AssetId, spectrum_document::RevisionId>,
}

#[derive(Clone)]
enum Durable {
    Save(AssetId, Box<Adjustments>),
    /// Step back or forward; the flag reloads the image afterward.
    Step(AssetId, bool, bool),
}

impl ImageEdits {
    /// Whether saves and history steps are all done.
    pub fn idle(&self) -> bool {
        self.pending.is_none() && self.queue.is_empty() && !self.busy
    }

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
