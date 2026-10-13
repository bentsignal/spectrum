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
            let (done, finished) = flume::bounded(1);
            std::thread::Builder::new()
                .name("spectrum-waveform".into())
                .spawn(move || {
                    crate::copies::background_priority();
                    let peaks = Service::open(&root).and_then(|service| {
                        let (media, file) = service.media(key.0)?;
                        let duration = media.info.duration;
                        let buckets = (duration.seconds() * PER_SECOND).ceil().max(1.) as usize;
                        spectrum_media::waveform(&file, key.1, duration, buckets)
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
        let header = crate::video_timeline::HEADER;
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
