//! Preview renders, as After Effects makes them: a stretch of the edit
//! (In to Out, or all of it) rendered ahead at the monitor's size and full
//! quality, then played from memory so it plays perfectly however heavy it
//! is. Frames are kept as JPEG and found by what makes them (the clips over
//! that moment, their settings and tracks, and the size), so an edit only
//! loses the frames it changes, and undoing it finds them again.
use image::RgbaImage;
use spectrum_assets::{Service, video_render::VideoSources};
use spectrum_media::Time;
use spectrum_video::{TrackKind, Video, render};
use std::{
    collections::HashMap,
    hash::{DefaultHasher, Hash, Hasher},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
};

/// The most memory rendered frames may take.
const BUDGET: usize = 2 << 30;
const QUALITY: u8 = 90;

/// Rendered frames, shared by the editor, its render, and the monitor.
#[derive(Default)]
pub struct Preview {
    frames: Mutex<HashMap<u64, Arc<Vec<u8>>>>,
    bytes: AtomicUsize,
    /// The render running now: frames done of all, and whether to stop.
    pub done: AtomicU64,
    pub total: AtomicU64,
    stop: Mutex<Option<Arc<AtomicBool>>>,
}

impl Preview {
    /// How many frames are rendered.
    pub fn len(&self) -> usize {
        self.frames.lock().map_or(0, |f| f.len())
    }

    pub fn has(&self, key: u64) -> bool {
        self.frames.lock().is_ok_and(|f| f.contains_key(&key))
    }

    /// A rendered frame, decoded, in the order it was rendered in.
    pub fn get(&self, key: u64) -> Option<RgbaImage> {
        let jpeg = self.frames.lock().ok()?.get(&key).cloned()?;
        let decoded = image::load_from_memory_with_format(&jpeg, image::ImageFormat::Jpeg).ok()?;
        Some(decoded.into_rgba8())
    }

    fn put(&self, key: u64, picture: &RgbaImage) -> bool {
        if self.bytes.load(Ordering::Relaxed) > BUDGET {
            return false;
        }
        // JPEG has no alpha; the monitor's frames are opaque.
        let rgb = image::DynamicImage::ImageRgba8(picture.clone()).into_rgb8();
        let mut jpeg = Vec::new();
        let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, QUALITY);
        if rgb.write_with_encoder(encoder).is_err() {
            return false;
        }
        self.bytes.fetch_add(jpeg.len(), Ordering::Relaxed);
        if let Ok(mut frames) = self.frames.lock() {
            frames.insert(key, Arc::new(jpeg));
        }
        true
    }

    /// Whether a render is running, and how far along it is.
    pub fn progress(&self) -> Option<f32> {
        let running = self
            .stop
            .lock()
            .ok()?
            .as_ref()
            .is_some_and(|s| !s.load(Ordering::Relaxed));
        let total = self.total.load(Ordering::Relaxed);
        (running && total > 0).then(|| self.done.load(Ordering::Relaxed) as f32 / total as f32)
    }

    pub fn cancel(&self) {
        if let Ok(stop) = self.stop.lock()
            && let Some(stop) = stop.as_ref()
        {
            stop.store(true, Ordering::Relaxed);
        }
    }

    /// Renders frames `from` to `to` of `video` at `size` in the background,
    /// skipping those already rendered; replaces any render running.
    pub fn render(
        self: &Arc<Self>,
        root: PathBuf,
        video: Video,
        size: (u32, u32),
        (from, to): (i64, i64),
    ) {
        self.cancel();
        let stop = Arc::new(AtomicBool::new(false));
        if let Ok(mut current) = self.stop.lock() {
            *current = Some(stop.clone());
        }
        self.done.store(0, Ordering::Relaxed);
        self.total
            .store((to - from).max(0) as u64, Ordering::Relaxed);
        let preview = self.clone();
        std::thread::Builder::new()
            .name("spectrum-preview".into())
            .spawn(move || {
                let rendered = (|| -> anyhow::Result<()> {
                    let service = Box::new(Service::open(&root)?);
                    let mut sources = VideoSources::new(service, true, true);
                    for index in from..to {
                        if stop.load(Ordering::Relaxed) {
                            return Ok(());
                        }
                        let key = frame_key(&video, size, index);
                        if !preview.has(key) {
                            let time = video.rate.time_of(index);
                            let picture = render::frame(
                                &video,
                                time,
                                size,
                                &mut sources,
                                render::Quality::Final,
                            )?;
                            if !preview.put(key, &picture) {
                                return Ok(());
                            }
                        }
                        preview.done.fetch_add(1, Ordering::Relaxed);
                    }
                    Ok(())
                })();
                if let Err(error) = rendered {
                    eprintln!("spectrum: preview render: {error:#}");
                }
                stop.store(true, Ordering::Relaxed);
            })
            .ok();
    }
}

/// What makes frame `index` of `video` at `size`: the picture clips over
/// it (all their settings), their tracks' order and visibility, the frame's
/// place in each, and the size.
pub fn frame_key(video: &Video, size: (u32, u32), index: i64) -> u64 {
    let time = video.rate.time_of(index);
    let mut hasher = DefaultHasher::new();
    (
        video.width,
        video.height,
        video.rate.fps().to_bits(),
        video.background,
        size,
    )
        .hash(&mut hasher);
    for (order, track) in video.tracks.iter().enumerate() {
        if track.kind != TrackKind::Video || track.muted {
            continue;
        }
        for clip in track.clips.iter().filter(|c| c.covers(time)) {
            (order, (time - clip.start).0).hash(&mut hasher);
            if let Ok(json) = serde_json::to_vec(clip) {
                json.hash(&mut hasher);
            }
        }
    }
    hasher.finish()
}

/// The frames a range covers: In to Out where set, otherwise the edit.
pub fn range(video: &Video, (in_point, out_point): (Option<Time>, Option<Time>)) -> (i64, i64) {
    let rate = video.rate;
    let last = crate::video_program::frames(video);
    let from = in_point.map_or(0, |t| rate.frame_at(t)).clamp(0, last);
    let to = out_point
        .map_or(last, |t| rate.frame_at(t))
        .clamp(from, last);
    (from, to)
}

impl crate::workspace::Workspace {
    /// I and O: In and Out at the playhead.
    pub fn set_in_point(&mut self, cx: &mut gpui::Context<Self>) {
        if let Some(video) = &mut self.video {
            video.in_point = Some(video.monitor.now());
            if video
                .out_point
                .is_some_and(|out| out <= video.monitor.now())
            {
                video.out_point = None;
            }
        }
        cx.notify();
    }

    pub fn set_out_point(&mut self, cx: &mut gpui::Context<Self>) {
        if let Some(video) = &mut self.video {
            video.out_point = Some(video.monitor.now());
            if video
                .in_point
                .is_some_and(|start| start >= video.monitor.now())
            {
                video.in_point = None;
            }
        }
        cx.notify();
    }

    pub fn clear_in_out(&mut self, cx: &mut gpui::Context<Self>) {
        if let Some(video) = &mut self.video {
            video.in_point = None;
            video.out_point = None;
        }
        cx.notify();
    }

    /// Enter: renders In to Out (or the whole edit) ahead, at the monitor's
    /// size, so it plays perfectly.
    pub fn render_video_preview(&mut self, cx: &mut gpui::Context<Self>) {
        let (Some(video), Ok(store)) = (&self.video, &self.store) else {
            return;
        };
        let size = crate::video_program::fitted(&video.doc, video.monitor.size);
        let frames = range(&video.doc, (video.in_point, video.out_point));
        video
            .preview
            .render(store.root.clone(), video.doc.clone(), size, frames);
        // Redraw while it renders, so the ruler and the note fill in.
        let preview = video.preview.clone();
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(250))
                    .await;
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    return;
                }
                if preview.progress().is_none() {
                    break;
                }
            }
        })
        .detach();
        cx.notify();
    }

    pub fn stop_preview(&mut self, cx: &mut gpui::Context<Self>) {
        if let Some(video) = &self.video {
            video.preview.cancel();
        }
        cx.notify();
    }
}

/// The rendered stretches across the ruler, as left and right points,
/// remembered until the edit, the view, or the frames change.
#[derive(Default)]
pub struct Strip {
    made: std::cell::RefCell<Option<(u64, Runs)>>,
}

/// Stretches across the timeline, left and right.
type Runs = Vec<(f32, f32)>;

impl crate::video_state::VideoEditor {
    pub fn rendered_strip(&self, width: f32) -> Vec<(f32, f32)> {
        if self.preview.len() == 0 {
            return Vec::new();
        }
        let size = crate::video_program::fitted(&self.doc, self.monitor.size);
        let mut hasher = DefaultHasher::new();
        (
            self.preview.len(),
            self.zoom.to_bits(),
            self.scroll.to_bits(),
            width.to_bits(),
            size,
        )
            .hash(&mut hasher);
        if let Ok(json) = serde_json::to_vec(&self.doc.tracks) {
            json.hash(&mut hasher);
        }
        let stamp = hasher.finish();
        if let Some((made, runs)) = self.strip.made.borrow().as_ref()
            && *made == stamp
        {
            return runs.clone();
        }
        let header = crate::video_layout::HEADER;
        let last = crate::video_program::frames(&self.doc);
        let mut runs: Vec<(f32, f32)> = Vec::new();
        let mut x = header;
        while x < width {
            let index = self.doc.rate.frame_at(self.time_at(x));
            if index < last && self.preview.has(frame_key(&self.doc, size, index)) {
                match runs.last_mut() {
                    Some(run) if run.1 >= x - 3.5 => run.1 = x + 3.,
                    _ => runs.push((x, x + 3.)),
                }
            }
            x += 3.;
        }
        *self.strip.made.borrow_mut() = Some((stamp, runs.clone()));
        runs
    }
}
