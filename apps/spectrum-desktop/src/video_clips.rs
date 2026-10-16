//! What clips look like on the timeline: a picture clip shows its asset's
//! picture at its head, and a sound clip its waveform (following the
//! clip's speed). Pictures come from the library's thumbnails; waveforms are
//! worked out once per sound track, in the background at low priority.
use crate::{
    store::Thumb,
    workspace::{Open, Workspace},
};
use gpui::{prelude::*, *};
use spectrum_assets::Service;
use spectrum_media::Time;
use spectrum_video::{Clip, Source};
use std::sync::Arc;

/// Peaks per second of sound.
const PER_SECOND: f64 = 50.;

/// A sound track's loudest sample in each fiftieth of a second.
pub struct Waveform {
    pub peaks: Vec<f32>,
}

impl Workspace {
    /// Asks for the pictures and waveforms the open video's clips show.
    pub fn request_clip_visuals(&mut self, cx: &mut Context<Self>) {
        let Some(video) = &self.video else {
            return;
        };
        let mut pictures = Vec::new();
        let mut sounds = Vec::new();
        for clip in video.doc.tracks.iter().flat_map(|t| t.clips.iter()) {
            match clip.source {
                Source::Sound { asset, track } => sounds.push((asset, track)),
                source => pictures.push(source.asset()),
            }
        }
        // The Media tab's tiles.
        if self.mode == crate::workspace::Mode::Media || video.adding.is_some() {
            pictures.extend(self.video_media(cx).into_iter().map(|a| a.id));
        }
        let Ok(store) = &mut self.store else {
            return;
        };
        for asset in pictures {
            if let std::collections::hash_map::Entry::Vacant(entry) = store.thumbs.entry(asset) {
                entry.insert(Thumb::Loading);
                store.thumb_queue.push_back(asset);
            }
        }
        self.render_next_thumbnail(cx);
        let Ok(store) = &mut self.store else {
            return;
        };
        let root = store.root.clone();
        for key in sounds {
            if store.waveforms.contains_key(&key) {
                continue;
            }
            store.waveforms.insert(key, None);
            let root = root.clone();
            // Waits while the program plays, as playback copies do.
            let hold = store.copies.hold.clone();
            let (done, finished) = flume::bounded(1);
            std::thread::Builder::new()
                .name("spectrum-waveform".into())
                .spawn(move || {
                    crate::copies::background_priority();
                    let peaks = Service::open(&root).and_then(|service| {
                        let (media, file) = service.media(key.0)?;
                        let duration = media.info.duration;
                        let buckets = (duration.seconds() * PER_SECOND).ceil().max(1.) as usize;
                        spectrum_media::waveform_with(&file, key.1, duration, buckets, &mut || {
                            while hold.load(std::sync::atomic::Ordering::Relaxed) {
                                std::thread::sleep(std::time::Duration::from_millis(50));
                            }
                        })
                    });
                    let _ = done.send(peaks);
                })
                .ok();
            cx.spawn(async move |this, cx| {
                let Ok(Ok(peaks)) = finished.recv_async().await else {
                    return;
                };
                this.update(cx, |this, cx| {
                    if let Ok(store) = &mut this.store {
                        store
                            .waveforms
                            .insert(key, Some(Arc::new(Waveform { peaks })));
                    }
                    cx.notify();
                })
                .ok();
            })
            .detach();
        }
    }

    /// The picture a clip shows at its head, once its thumbnail is ready.
    pub fn clip_picture(&self, clip: &Clip) -> Option<std::path::PathBuf> {
        if !matches!(self.open, Open::Video(_)) || matches!(clip.source, Source::Sound { .. }) {
            return None;
        }
        match self.store.as_ref().ok()?.thumbs.get(&clip.source.asset())? {
            Thumb::Ready(path, _) => Some(path.clone()),
            _ => None,
        }
    }

    /// A sound clip's waveform painted across its box, `left` to `right`
    /// on the timeline and `top` down `height`.
    pub fn clip_waveform(
        &self,
        clip: &Clip,
        (left, top, right, height): (f32, f32, f32, f32),
    ) -> Option<AnyElement> {
        let Source::Sound { asset, track } = clip.source else {
            return None;
        };
        let waveform = self
            .store
            .as_ref()
            .ok()?
            .waveforms
            .get(&(asset, track))?
            .clone()?;
        let video = self.video.as_ref()?;
        let (zoom, scroll) = (video.zoom, video.scroll);
        let clip = clip.clone();
        let header = crate::video_layout::HEADER;
        // Only the part on screen, drawn as one shape: a clip far wider
        // than the timeline would otherwise paint thousands of pieces each
        // time the playhead moves.
        let width = f32::from(video.timeline.borrow().size.width);
        let (left, right) = (left.max(header), right.min(width));
        if right <= left {
            return None;
        }
        Some(
            canvas(
                |_, _, _| {},
                move |bounds, _, window, _| {
                    let middle = top + height / 2.;
                    let reach = height / 2. - 2.;
                    let mut halves = Vec::new();
                    let mut x = left;
                    while x <= right {
                        // The source this column of the timeline plays.
                        let time = Time::from_seconds(scroll + f64::from((x - header) / zoom));
                        let source = clip.source_seconds(time);
                        let index = (source * PER_SECOND) as usize;
                        let peak = waveform.peaks.get(index).copied().unwrap_or(0.).sqrt();
                        halves.push((x, (peak * reach).max(0.5)));
                        x += 2.;
                    }
                    let at = |x: f32, y: f32| bounds.origin + point(px(x), px(y));
                    let mut shape = PathBuilder::fill();
                    for (i, (x, half)) in halves.iter().enumerate() {
                        if i == 0 {
                            shape.move_to(at(*x, middle - half));
                        } else {
                            shape.line_to(at(*x, middle - half));
                        }
                    }
                    for (x, half) in halves.iter().rev() {
                        shape.line_to(at(*x, middle + half));
                    }
                    shape.close();
                    if let Ok(path) = shape.build() {
                        window.paint_path(path, rgba(0xa8d8b980));
                    }
                },
            )
            .absolute()
            .size_full()
            .into_any_element(),
        )
    }
}

/// The share of a fade done at `u`, eased as the engine eases it.
fn ease(u: f32) -> f32 {
    let u = u.clamp(0., 1.);
    u * u * (3. - 2. * u)
}

impl Workspace {
    /// A clip's fades drawn over its box: the faded part darkened under a
    /// curve, a crossfade as an X, and (for the selected clip) the handles
    /// that drag them. `(left, top, right, height)` in timeline points.
    pub fn clip_fades(
        &self,
        clip: &Clip,
        (left, top, right, height): (f32, f32, f32, f32),
        selected: bool,
    ) -> Option<AnyElement> {
        let video = self.video.as_ref()?;
        if !clip.has_fades() && !selected {
            return None;
        }
        let in_x = video.x_of(clip.start + clip.fade_in);
        let out_x = video.x_of(clip.end() - clip.fade_out);
        let crossfade = clip.crossfade && clip.fade_in.0 > 0;
        let (fade_in, fade_out) = (clip.fade_in.0 > 0, clip.fade_out.0 > 0);
        let header = crate::video_layout::HEADER;
        Some(
            canvas(
                |_, _, _| {},
                move |area, _, window, _| {
                    let at = |x: f32, y: f32| area.origin + point(px(x), px(y));
                    let bottom = top + height;
                    // A curve from (x0, bottom) rising to (x1, top), sampled.
                    let curve = |x0: f32, x1: f32, rising: bool| -> Vec<(f32, f32)> {
                        (0..=16)
                            .map(|i| {
                                let u = i as f32 / 16.;
                                let share = if rising { ease(u) } else { 1. - ease(u) };
                                (x0 + (x1 - x0) * u, bottom - share * height)
                            })
                            .collect()
                    };
                    let shade =
                        |window: &mut Window, points: Vec<(f32, f32)>, corner: (f32, f32)| {
                            let mut path = PathBuilder::fill();
                            path.move_to(at(corner.0, corner.1));
                            for (x, y) in &points {
                                path.line_to(at(x.max(header), *y));
                            }
                            path.close();
                            if let Ok(path) = path.build() {
                                window.paint_path(path, rgba(0x00000070));
                            }
                        };
                    let stroke = |window: &mut Window, points: &[(f32, f32)]| {
                        let mut path = PathBuilder::stroke(px(1.));
                        for (i, (x, y)) in points.iter().enumerate() {
                            if i == 0 {
                                path.move_to(at(x.max(header), *y));
                            } else {
                                path.line_to(at(x.max(header), *y));
                            }
                        }
                        if let Ok(path) = path.build() {
                            window.paint_path(path, rgba(0xffffffb0));
                        }
                    };
                    if fade_in {
                        let rising = curve(left, in_x, true);
                        shade(window, rising.clone(), (left, top));
                        stroke(window, &rising);
                        if crossfade {
                            // The clip before fading out: the other arm of the X.
                            stroke(window, &curve(left, in_x, false));
                        }
                    }
                    if fade_out {
                        let falling = curve(out_x, right, false);
                        shade(window, falling.clone(), (right, top));
                        stroke(window, &falling);
                    }
                    if selected {
                        for x in [
                            if fade_in { in_x } else { left + 4. },
                            if fade_out { out_x } else { right - 4. },
                        ] {
                            if x >= header {
                                window.paint_quad(fill(
                                    Bounds::new(at(x - 3., top), size(px(6.), px(6.))),
                                    rgb(0xececec),
                                ));
                            }
                        }
                    }
                },
            )
            .absolute()
            .size_full()
            .into_any_element(),
        )
    }
}
