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
/// Quiet time after a drag before its full-size frame renders, and the
/// most it backs off to when drags resume before those frames are in.
const SHARP_AFTER: Duration = Duration::from_millis(60);
const SHARP_AFTER_MOST: Duration = Duration::from_millis(180);
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
    /// The adjustments `image` was rendered with: `shown` is set when its
    /// render starts.
    pub displayed: Option<Adjustments>,
    /// The other view's last full-size frame (the whole frame while not
    /// cropping, the cropped picture while cropping) and its adjustments, so
    /// switching between Crop and the other modes shows it at once.
    pub other: Option<(Adjustments, Arc<RenderImage>)>,
    /// Whether the latest render is full size.
    sharp: bool,
    /// Full-size frames rendering after a drag (`sharpen_frame`), and the
    /// edits each is for.
    sharpening: Vec<Adjustments>,
    /// The quiet time a drag waits before its full-size frame: longer each
    /// time the drag went on and one was let go, back to the shortest once
    /// one shows.
    sharp_after: Duration,
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

impl Preview {
    /// Notes what a frame at `level` cost and that it was a full or draft
    /// frame. A fresh full-size cost re-estimates the smaller levels, as
    /// edits may have grown lighter or heavier since they ran.
    fn record_cost(&mut self, level: usize, cost: Duration) {
        if level == 0 {
            self.costs = [None; LEVELS.len()];
        }
        self.costs[level] = Some(cost);
        crate::perf::record(
            if level == 0 {
                "image_preview_render"
            } else {
                "image_draft_render"
            },
            cost,
        );
    }
}

/// The threads that render frames for edits on screen. Full-size frames
/// after a drag render on the shared threads, so a drag's next frame never
/// waits in line behind one: the system shares the cores between them.
fn frames() -> &'static rayon::ThreadPool {
    static FRAMES: std::sync::OnceLock<rayon::ThreadPool> = std::sync::OnceLock::new();
    FRAMES.get_or_init(|| {
        rayon::ThreadPoolBuilder::new()
            .thread_name(|index| format!("frames-{index}"))
            .build()
            .expect("a thread pool for frames")
    })
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
        for pixel in chunk.as_chunks_mut::<4>().0 {
            pixel.swap(0, 2);
        }
    });
    Arc::new(RenderImage::new(vec![image::Frame::new(pixels)]))
}

/// The part of a rendered frame inside `crop` (in shares of the frame),
/// as its own frame: a cropped picture straight from the whole one.
fn cut(image: &RenderImage, crop: spectrum_image::CropRect) -> Option<Arc<RenderImage>> {
    let size = image.size(0);
    let (width, height) = (size.width.0 as u32, size.height.0 as u32);
    let bytes = image.as_bytes(0)?;
    let x = ((crop.x * width as f32).round() as u32).min(width.saturating_sub(1));
    let y = ((crop.y * height as f32).round() as u32).min(height.saturating_sub(1));
    let w = ((crop.width * width as f32).round() as u32).clamp(1, width - x);
    let h = ((crop.height * height as f32).round() as u32).clamp(1, height - y);
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for row in y..y + h {
        let start = ((row * width + x) * 4) as usize;
        out.extend_from_slice(&bytes[start..start + (w * 4) as usize]);
    }
    // Already in the frame's byte order, so it is wrapped as it is.
    let pixels = image::RgbaImage::from_raw(w, h, out)?;
    Some(Arc::new(RenderImage::new(vec![image::Frame::new(pixels)])))
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

    /// Whether the open image shows its current edits at full size, saved
    /// or not.
    pub fn image_idle_view(&self) -> bool {
        let wanted = self.view_adjustments();
        self.image.preview.as_ref().is_some_and(|preview| {
            preview.image.is_some()
                && preview.sharp
                && !preview.rendering
                && preview.shown.as_ref() == Some(&wanted)
        })
    }

    /// What the main area shows: the whole frame while cropping.
    pub fn view_adjustments(&self) -> Adjustments {
        if self.mode == Mode::Crop {
            Adjustments {
                crop: None,
                ..self.image.adjust.clone()
            }
        } else {
            self.image.adjust.clone()
        }
    }

    /// Switching to or from Crop changes whether the view shows the whole
    /// frame. What can show at once does: the other view's last frame if it
    /// still matches (sharp, nothing to render), or else, coming back from
    /// Crop with a new crop, that crop cut from the whole frame on screen.
    /// The exact frame renders after. The crop view draws a cropped frame
    /// where its crop sits until the whole frame arrives.
    pub fn switch_image_view(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let wanted = self.view_adjustments();
        let Some(preview) = &mut self.image.preview else {
            return;
        };
        if preview.displayed.as_ref() == Some(&wanted) {
            return self.render_preview(window, cx);
        }
        let current = preview.displayed.clone().zip(preview.image.clone());
        // Only a full-size frame is kept to show sharp later.
        let keep = current.clone().filter(|_| preview.sharp);
        let stored = preview.other.take();
        match stored {
            Some((adjustments, image)) if adjustments == wanted => {
                preview.image = Some(image);
                preview.displayed = Some(wanted.clone());
                preview.shown = Some(wanted.clone());
                preview.requested = Some(wanted);
                preview.sharp = true;
                preview.other = keep;
            }
            stored => {
                if let Some((_, image)) = stored {
                    crate::retire::drop_later(image);
                }
                let whole = current
                    .as_ref()
                    .filter(|(shown, _)| {
                        shown.crop.is_none()
                            && Adjustments {
                                crop: None,
                                ..wanted.clone()
                            } == *shown
                    })
                    .and_then(|(_, image)| Some(cut(image, wanted.crop?)));
                if let Some(cropped) = whole.flatten() {
                    preview.image = Some(cropped);
                    preview.displayed = Some(wanted.clone());
                    preview.shown = Some(wanted.clone());
                    preview.sharp = false;
                }
                preview.other = keep;
            }
        }
        self.render_preview(window, cx);
        cx.notify();
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
            let other = old.other.map(|(_, image)| image);
            for image in old.image.into_iter().chain(old.original).chain(other) {
                crate::retire::drop_later(image);
            }
        }
        self.image.view.reset();
        self.image.preview = Some(Preview {
            id,
            sources: None,
            image: None,
            shown: None,
            displayed: None,
            other: None,
            sharp: false,
            sharpening: Vec::new(),
            sharp_after: SHARP_AFTER,
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
        let now = Instant::now();
        if preview.requested.as_ref() != Some(&wanted) {
            preview.edit_gap = preview.last_edit.map_or(Duration::MAX, |last| now - last);
            preview.last_edit = Some(now);
            preview.requested = Some(wanted.clone());
        }
        let current = preview.shown.as_ref() == Some(&wanted);
        if !current {
            preview.unseen_since.get_or_insert(now);
            if preview.rendering {
                return;
            }
            // A drag shows frames at the size this computer keeps up with.
            let level = if preview.edit_gap < DRAG_GAP {
                drag_level(&preview.costs)
            } else {
                0
            };
            return self.render_frame(wanted, level, window, cx);
        }
        // One already rendering these edits is enough; one for edits since
        // replaced is let go, and two at once at most.
        if preview.sharp || preview.sharpening.contains(&wanted) || preview.sharpening.len() >= 2 {
            return;
        }
        // The full-size frame after a drag waits for the drag to pause.
        let wait = preview.sharp_after;
        let quiet = preview.last_edit.map_or(wait, |last| now - last);
        if quiet < wait {
            cx.spawn_in(window, async move |this, cx| {
                cx.background_executor().timer(wait - quiet).await;
                this.update_in(cx, |this, window, cx| this.render_preview(window, cx))
                    .ok();
            })
            .detach();
            return;
        }
        self.sharpen_frame(wanted, window, cx);
    }

    /// Renders a frame for edits not yet on screen, one at a time; the next
    /// starts when it is in.
    fn render_frame(
        &mut self,
        wanted: Adjustments,
        level: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(preview) = &mut self.image.preview else {
            return;
        };
        let Some(sources) = preview.sources.clone() else {
            return;
        };
        preview.rendering = true;
        preview.shown = Some(wanted.clone());
        let for_display = wanted.clone();
        let covers = preview.unseen_since.take();
        let edited = preview.last_edit;
        let id = preview.id;
        let task = cx.background_executor().spawn(async move {
            let started = Instant::now();
            let rendered = frames().install(|| render(&sources.levels[level], wanted));
            (rendered, started.elapsed())
        });
        cx.spawn_in(window, async move |this, cx| {
            let (rendered, cost) = task.await;
            this.update_in(cx, |this, window, cx| {
                let Some(preview) = this.image.preview.as_mut().filter(|p| p.id == id) else {
                    crate::retire::drop_later(rendered.image);
                    return;
                };
                preview.rendering = false;
                let draft = level > 0;
                preview.sharp = !draft;
                preview.record_cost(level, cost);
                if let Some(old) = preview.image.replace(rendered.image) {
                    crate::retire::drop_later(old);
                }
                preview.displayed = Some(for_display);
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
                // Zoomed in, the part on screen renders sharp for these edits.
                if !draft {
                    this.refresh_image_detail(window, cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Renders the full-size frame for the edits on screen as a draft once
    /// a drag pauses. It never holds up the drag's frames: if the drag goes
    /// on meanwhile, they render beside it and this one is let go.
    fn sharpen_frame(&mut self, wanted: Adjustments, window: &mut Window, cx: &mut Context<Self>) {
        let Some(preview) = &mut self.image.preview else {
            return;
        };
        let Some(sources) = preview.sources.clone() else {
            return;
        };
        preview.sharpening.push(wanted.clone());
        let (id, edited) = (preview.id, preview.last_edit);
        let task = cx.background_executor().spawn({
            let wanted = wanted.clone();
            async move {
                let started = Instant::now();
                let rendered = render(&sources.levels[0], wanted);
                (rendered, started.elapsed())
            }
        });
        cx.spawn_in(window, async move |this, cx| {
            let (rendered, cost) = task.await;
            this.update_in(cx, |this, window, cx| {
                let Some(preview) = this.image.preview.as_mut().filter(|p| p.id == id) else {
                    crate::retire::drop_later(rendered.image);
                    return;
                };
                if let Some(index) = preview.sharpening.iter().position(|w| *w == wanted) {
                    preview.sharpening.remove(index);
                }
                preview.record_cost(0, cost);
                let still = preview.shown.as_ref() == Some(&wanted)
                    && preview.requested.as_ref() == Some(&wanted);
                if !still {
                    preview.sharp_after = (preview.sharp_after * 2).min(SHARP_AFTER_MOST);
                    crate::retire::drop_later(rendered.image);
                    return this.render_preview(window, cx);
                }
                preview.sharp_after = SHARP_AFTER;
                preview.sharp = true;
                if let Some(old) = preview.image.replace(rendered.image) {
                    crate::retire::drop_later(old);
                }
                preview.displayed = Some(wanted);
                preview.histogram = Some(Arc::new(rendered.histogram));
                if let Some(edited) = edited {
                    crate::perf::record("image_sharp", edited.elapsed());
                }
                this.refresh_image_detail(window, cx);
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
