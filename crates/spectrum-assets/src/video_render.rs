//! Where a video's pictures and sound come from: footage (from its playback
//! copy while editing, from the original for export), images and canvases
//! drawn once at the size they show, and sound tracks read straight on.
use super::Service;
use anyhow::{Context, Result};
use image::RgbaImage;
use spectrum_library::AssetId;
use spectrum_media::{AudioReader, ExportSettings, FrameReader, Media, Writer};
use spectrum_time::Time;
use spectrum_video::{Clip, Interpolation, Source, render};
use std::{collections::HashMap, path::PathBuf, sync::Arc};

/// A footage frame by asset, frame number, and the box it fits.
type FrameKey = (AssetId, i64, (u32, u32));

/// Sources for drawing and mixing one video, reading the library through
/// `S`: a borrowed service, or one the sources own on another thread.
pub struct VideoSources<S: std::ops::Deref<Target = Service>> {
    service: S,
    /// Read footage's playback copies where they exist.
    copies: bool,
    /// Pictures in blue, green, red, alpha order, as the screen takes them.
    bgra: bool,
    media: HashMap<AssetId, (Media, PathBuf, Option<PathBuf>)>,
    readers: HashMap<AssetId, FrameReader>,
    stills: HashMap<(AssetId, (u32, u32)), Arc<RgbaImage>>,
    /// Each sound clip's reader and where it will read next.
    sounds: HashMap<u64, (AudioReader, Time)>,
    /// A second of each sped-up, slowed, or reversed sound clip's source,
    /// decoded: its first sample and the samples. Playing backward reads
    /// from here instead of seeking for every step.
    windows: HashMap<u64, (i64, Vec<f32>)>,
    /// Sound readers opened, which seeks cost.
    opened: usize,
    scratch: Vec<f32>,
    /// Footage frames decoded lately, newest last: slow motion and blends
    /// ask for the same few frames again and again, often out of order,
    /// and playing backward takes them from a group decoded at once.
    recent: Vec<(FrameKey, Arc<RgbaImage>)>,
    /// The frame each footage was last asked for, to tell playing
    /// backward.
    last: HashMap<AssetId, i64>,
    /// The last pair of frames motion was worked out between: slow motion
    /// draws many moments of one pair.
    motion: Option<(FrameKey, spectrum_imaging::flow::MotionPair)>,
}

impl<S: std::ops::Deref<Target = Service>> VideoSources<S> {
    pub fn new(service: S, copies: bool, bgra: bool) -> Self {
        Self {
            service,
            copies,
            bgra,
            media: HashMap::new(),
            readers: HashMap::new(),
            stills: HashMap::new(),
            sounds: HashMap::new(),
            windows: HashMap::new(),
            opened: 0,
            scratch: Vec::new(),
            recent: Vec::new(),
            last: HashMap::new(),
            motion: None,
        }
    }

    fn media(&mut self, asset: AssetId) -> Result<&(Media, PathBuf, Option<PathBuf>)> {
        if !self.media.contains_key(&asset) {
            let (media, file) = self.service.media(asset)?;
            let copy = if self.copies {
                self.service.playback_copy(asset)?
            } else {
                None
            };
            self.media.insert(asset, (media, file, copy));
        }
        self.media.get(&asset).context("no media")
    }

    /// Forgets stills drawn earlier, as after an image or canvas changes.
    pub fn forget_stills(&mut self) {
        self.stills.clear();
    }

    /// Footage's frame `index`, fitted inside `fit`, from the recent frames
    /// when it is one.
    fn footage_frame(
        &mut self,
        asset: AssetId,
        index: i64,
        fit: (u32, u32),
    ) -> Result<Arc<RgbaImage>> {
        let key = (asset, index, fit);
        let previous = self.last.insert(asset, index);
        if let Some((_, frame)) = self.recent.iter().find(|(k, _)| *k == key) {
            return Ok(frame.clone());
        }
        // A frame or a few before the last: playing backward.
        let backward = previous.is_some_and(|last| index < last && last - index <= 4);
        let frames = self.decode_frames(asset, index, fit, backward)?;
        let mut wanted = None;
        for (at, frame) in frames {
            let frame = Arc::new(frame);
            if at == index {
                wanted = Some(frame.clone());
            }
            self.keep((asset, at, fit), frame);
        }
        wanted.context("the frame was not decoded")
    }

    /// Keeps a decoded frame among the recent ones, forgetting the oldest
    /// past a few frames and a memory budget.
    fn keep(&mut self, key: FrameKey, frame: Arc<RgbaImage>) {
        const FEWEST: usize = 4;
        const BUDGET: usize = 512 << 20;
        self.recent.retain(|(k, _)| *k != key);
        self.recent.push((key, frame));
        let mut bytes: usize = self.recent.iter().map(|(_, f)| f.as_raw().len()).sum();
        while self.recent.len() > FEWEST && bytes > BUDGET {
            bytes -= self.recent.remove(0).1.as_raw().len();
        }
    }

    /// Frame `index`, or (playing backward) the frames from the keyframe
    /// before it up to it, by frame number.
    fn decode_frames(
        &mut self,
        asset: AssetId,
        index: i64,
        fit: (u32, u32),
        backward: bool,
    ) -> Result<Vec<(i64, RgbaImage)>> {
        /// Frames kept from a group decoded for playing backward.
        const BACK: usize = 60;
        let (media, file, copy) = self.media(asset)?.clone();
        let video = media.info.video.context("footage has a picture")?;
        if !self.readers.contains_key(&asset) {
            let reader = match &copy {
                Some(copy) => FrameReader::open(copy)?.with_rotation(video.rotation),
                None => FrameReader::open(&file)?,
            };
            let reader = if self.bgra { reader.bgra() } else { reader };
            self.readers.insert(asset, reader);
        }
        let (width, height) = fitted((video.width, video.height), fit);
        let reader = self.readers.get_mut(&asset).context("no reader")?;
        let last = reader.frames().max(1) as i64 - 1;
        let clamped = index.clamp(0, last);
        let long = Some(width.max(height));
        if backward {
            let frames = reader.frames_before(clamped, BACK, long)?;
            if frames.iter().any(|f| f.index == clamped) {
                return Ok(frames
                    .into_iter()
                    .map(|f| {
                        // Asked-for frames past the end show the last.
                        let at = if f.index == clamped { index } else { f.index };
                        (at, scale_to(f.image, (width, height)))
                    })
                    .collect());
            }
        }
        let frame = reader.frame_at(video.rate.time_of(clamped), long)?;
        Ok(vec![(index, scale_to(frame.image, (width, height)))])
    }

    /// Footage at exact source seconds: the frame showing then, or (for a
    /// sped-up or slowed clip between source frames) one made between the
    /// two nearest, as the clip asks.
    fn footage(
        &mut self,
        clip: &Clip,
        asset: AssetId,
        source: f64,
        fit: (u32, u32),
        quality: render::Quality,
    ) -> Result<Arc<RgbaImage>> {
        let rate = self
            .media(asset)?
            .0
            .info
            .video
            .as_ref()
            .context("footage has a picture")?
            .rate;
        let position = source * rate.fps();
        let index = (position + 1e-6).floor() as i64;
        let between = (position - index as f64) as f32;
        let method = match (clip.interpolation, quality) {
            _ if clip.speed.is_normal() || between < 0.01 => Interpolation::Off,
            (Interpolation::Motion, render::Quality::Playback) => Interpolation::Blend,
            (method, _) => method,
        };
        if method == Interpolation::Off {
            return self.footage_frame(asset, index, fit);
        }
        let key = (asset, index, fit);
        if method == Interpolation::Motion
            && let Some((cached, pair)) = &self.motion
            && *cached == key
        {
            return Ok(Arc::new(pair.at(between)));
        }
        let first = self.footage_frame(asset, index, fit)?;
        let second = self.footage_frame(asset, index + 1, fit)?;
        if method == Interpolation::Blend {
            return Ok(Arc::new(spectrum_imaging::flow::blend(
                &first, &second, between,
            )));
        }
        let pair = spectrum_imaging::flow::MotionPair::new((*first).clone(), (*second).clone());
        let frame = pair.at(between);
        self.motion = Some((key, pair));
        Ok(Arc::new(frame))
    }

    fn still(&mut self, source: Source, fit: (u32, u32)) -> Result<Arc<RgbaImage>> {
        let asset = source.asset();
        if let Some(still) = self.stills.get(&(asset, fit)) {
            return Ok(still.clone());
        }
        // Up to 2048 pixels, stills come from the asset's own cached render
        // at the next size up, which decodes the source at reduced size;
        // only larger exports render it whole.
        let long = fit.0.max(fit.1);
        let picture = match ([512, 1024, 2048].into_iter().find(|b| *b >= long), source) {
            (Some(size), _) => image::open(self.service.thumbnail(asset, size)?)?.into_rgba8(),
            (None, Source::Image { .. }) => image::open(self.service.preview(asset)?)?.into_rgba8(),
            (None, _) => {
                let document = self.service.canvas(asset)?;
                spectrum_canvas::render_document(&document, Some(long))?.into_rgba8()
            }
        };
        let size = fitted(picture.dimensions(), fit);
        let mut picture = scale_to(picture, size);
        if self.bgra {
            for pixel in picture.pixels_mut() {
                pixel.0.swap(0, 2);
            }
        }
        let picture = Arc::new(picture);
        self.stills.insert((asset, fit), picture.clone());
        Ok(picture)
    }
}

/// The largest size with `source`'s shape that fits inside `fit`.
fn fitted((width, height): (u32, u32), (fit_width, fit_height): (u32, u32)) -> (u32, u32) {
    let scale = (f64::from(fit_width) / f64::from(width.max(1)))
        .min(f64::from(fit_height) / f64::from(height.max(1)));
    (
        ((f64::from(width) * scale).round() as u32).clamp(1, fit_width),
        ((f64::from(height) * scale).round() as u32).clamp(1, fit_height),
    )
}

/// `picture` at `size`, resized only when it differs.
fn scale_to(picture: RgbaImage, size: (u32, u32)) -> RgbaImage {
    if picture.dimensions() == size {
        picture
    } else {
        image::imageops::resize(
            &picture,
            size.0,
            size.1,
            image::imageops::FilterType::Triangle,
        )
    }
}

impl<S: std::ops::Deref<Target = Service>> render::Pictures for VideoSources<S> {
    fn picture(
        &mut self,
        clip: &Clip,
        source: f64,
        fit: (u32, u32),
        quality: render::Quality,
    ) -> Result<Option<Arc<RgbaImage>>> {
        Ok(Some(match clip.source {
            Source::Footage { asset } => self.footage(clip, asset, source, fit, quality)?,
            Source::Image { .. } | Source::Canvas { .. } => self.still(clip.source, fit)?,
            Source::Sound { .. } => return Ok(None),
        }))
    }
}

impl<S: std::ops::Deref<Target = Service>> render::Sounds for VideoSources<S> {
    fn mix(&mut self, clip: &Clip, from: Time, gain: f32, out: &mut [f32]) -> Result<()> {
        let Source::Sound { .. } = clip.source else {
            return Ok(());
        };
        if clip.speed.is_normal() || !clip.sound_follows_speed {
            // Normal speed: straight from the source, from the in point.
            let at = clip.source_in + (from - clip.start);
            let at = Time::from_seconds(clip.wrap(at.seconds()));
            return self.read_into(clip, at, gain, out);
        }
        // At speed, in short steps: each reads the stretch of source it
        // covers and stretches or squeezes it to fit (so the pitch follows
        // the speed), backward when the speed is, and silent when frozen.
        const STEP: usize = 256;
        let frames = out.len() / 2;
        let mut done = 0;
        while done < frames {
            let count = STEP.min(frames - done);
            let at = |i: usize| from + Time::from_samples(i as i64, 48_000);
            let a = clip.unwrapped_seconds(at(done));
            let b = clip.unwrapped_seconds(at(done + count));
            let span = ((b - a).abs() * 48_000.).round() as usize;
            if span >= 1 {
                let start = clip.wrap(a.min(b));
                let mut piece = vec![0f32; (span + 1) * 2];
                self.read_window(clip, start, b < a, &mut piece)?;
                if b < a {
                    piece.reverse_by_frames();
                }
                for i in 0..count {
                    let position = i as f64 * span as f64 / count as f64;
                    let k = (position.floor() as usize).min(span);
                    let f = (position - k as f64) as f32;
                    let next = (k + 1).min(span);
                    for c in 0..2 {
                        let value = piece[k * 2 + c] * (1. - f) + piece[next * 2 + c] * f;
                        out[(done + i) * 2 + c] += value * gain;
                    }
                }
            }
            done += count;
        }
        Ok(())
    }
}

impl<S: std::ops::Deref<Target = Service>> VideoSources<S> {
    /// Adds `out.len() / 2` sample frames of a sound clip's source from
    /// `at` into `out` at `gain`, reading straight on when it continues
    /// where the last read ended.
    fn read_into(&mut self, clip: &Clip, at: Time, gain: f32, out: &mut [f32]) -> Result<()> {
        let Source::Sound { asset, track } = clip.source else {
            return Ok(());
        };
        let file = self.media(asset)?.1.clone();
        let continues = self
            .sounds
            .get(&clip.id)
            .is_some_and(|(_, next)| (at - *next).0.abs() < Time::from_samples(2, 48_000).0);
        if !continues {
            let mut reader = AudioReader::open(&file, track)?;
            reader.seek(at)?;
            self.opened += 1;
            self.sounds.insert(clip.id, (reader, at));
        }
        let (reader, next) = self.sounds.get_mut(&clip.id).context("no reader")?;
        self.scratch.resize(out.len(), 0.);
        let read = reader.read(&mut self.scratch)?;
        *next = at + Time::from_samples(read as i64, 48_000);
        for (sample, add) in out.iter_mut().zip(&self.scratch[..read * 2]) {
            *sample += add * gain;
        }
        Ok(())
    }
}

impl<S: std::ops::Deref<Target = Service>> VideoSources<S> {
    /// How many sound readers have been opened, each a seek.
    pub fn sound_opens(&self) -> usize {
        self.opened
    }

    /// Fills `out` with a sound clip's source from `start` seconds, from a
    /// decoded second around it: ahead of it going forward, behind it going
    /// backward, so playing backward seeks about once a second.
    fn read_window(
        &mut self,
        clip: &Clip,
        start: f64,
        backward: bool,
        out: &mut [f32],
    ) -> Result<()> {
        const WINDOW: i64 = 48_000;
        let first = (start * 48_000.).round() as i64;
        let frames = (out.len() / 2) as i64;
        let covered = self.windows.get(&clip.id).is_some_and(|(at, samples)| {
            first >= *at && first + frames <= at + (samples.len() / 2) as i64
        });
        if !covered {
            let from = if backward {
                (first + frames - WINDOW).max(0)
            } else {
                first
            };
            let length = WINDOW.max(frames);
            let mut samples = vec![0f32; length as usize * 2];
            self.read_into(clip, Time::from_samples(from, 48_000), 1., &mut samples)?;
            self.windows.insert(clip.id, (from, samples));
        }
        let (at, samples) = self.windows.get(&clip.id).context("no window")?;
        let offset = ((first - at).max(0) * 2) as usize;
        let end = (offset + out.len()).min(samples.len());
        if offset < end {
            for (sample, add) in out.iter_mut().zip(&samples[offset..end]) {
                *sample += add;
            }
        }
        Ok(())
    }
}

/// Reverses interleaved stereo by sample frames, keeping left and right.
trait ReverseFrames {
    fn reverse_by_frames(&mut self);
}

impl ReverseFrames for Vec<f32> {
    fn reverse_by_frames(&mut self) {
        let frames = self.len() / 2;
        for i in 0..frames / 2 {
            let j = frames - 1 - i;
            self.swap(i * 2, j * 2);
            self.swap(i * 2 + 1, j * 2 + 1);
        }
    }
}

impl Service {
    /// A small picture of a video for grids and history: a frame a third of
    /// the way in, no larger than `max` on its long edge.
    pub fn video_poster(&self, video: &spectrum_video::Video, max: u32) -> Result<RgbaImage> {
        let size = fitted((video.width, video.height), (max, max));
        let time = video.snap(Time(video.duration().0 / 3));
        render::frame(
            video,
            time,
            size,
            &mut VideoSources::new(self, true, false),
            render::Quality::Playback,
        )
    }

    /// The frame of a video showing at `time`, at `size`.
    pub fn video_frame(&self, id: AssetId, time: Time, size: (u32, u32)) -> Result<RgbaImage> {
        let video = self.video(id)?;
        render::frame(
            &video,
            time,
            size,
            &mut VideoSources::new(self, false, false),
            render::Quality::Final,
        )
    }

    /// Writes a video to a file outside the library. `progress` hears the
    /// share done and stops the export by returning false.
    pub fn export_video(
        &self,
        id: AssetId,
        path: &std::path::Path,
        settings: ExportSettings,
        progress: &mut dyn FnMut(f32) -> bool,
    ) -> Result<String> {
        self.export_video_range(id, path, settings, None, progress)
    }

    /// Exports `from` to `to` of the video (all of it when `None`).
    pub fn export_video_range(
        &self,
        id: AssetId,
        path: &std::path::Path,
        settings: ExportSettings,
        range: Option<(Time, Time)>,
        progress: &mut dyn FnMut(f32) -> bool,
    ) -> Result<String> {
        self.check_export(path)?;
        let video = self.video(id)?;
        let duration = video.duration();
        if duration == Time::ZERO {
            anyhow::bail!("the video is empty");
        }
        let rate = settings.rate;
        let (from, to) = range.unwrap_or((Time::ZERO, duration));
        let to = to.min(duration);
        if to <= from {
            anyhow::bail!("the range to export is empty");
        }
        let first = rate.frame_at(from);
        let frames = (rate.frame_at(to - Time(1)) + 1 - first).max(1);
        let mut sources = VideoSources::new(self, false, false);
        let mut writer = Writer::create(path, settings)?;
        let encoder = writer.encoder.to_string();
        let size = (settings.width, settings.height);
        let mut sound = Vec::new();
        for index in 0..frames {
            let time = rate.time_of(first + index);
            let picture = render::frame(&video, time, size, &mut sources, render::Quality::Final)?;
            writer.push_frame(&picture)?;
            if settings.sound {
                let samples =
                    rate.time_of(first + index + 1).samples(48_000) - time.samples(48_000);
                sound.resize(samples as usize * 2, 0.);
                render::sound(&video, time, &mut sound, &mut sources)?;
                writer.push_sound(&sound)?;
            }
            if index % 15 == 0 && !progress(index as f32 / frames as f32) {
                drop(writer);
                let _ = std::fs::remove_file(path);
                anyhow::bail!("the export was stopped");
            }
        }
        writer.finish()?;
        progress(1.);
        Ok(encoder)
    }
}
