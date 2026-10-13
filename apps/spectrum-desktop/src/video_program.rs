//! The video editor's program: its frames, drawn from the clips at each
//! time from footage's playback copies, and its sound, mixed from every
//! audio track. Both live on the player's threads and take each edit as it
//! is made.
use crate::{audio_out::Sounds, player_decode::Pictures};
use image::RgbaImage;
use spectrum_assets::{Service, video_render::VideoSources};
use spectrum_media::{FrameRate, Time};
use spectrum_video::{Video, render};
use std::path::PathBuf;

/// How many frames a video has: every frame that starts before its end,
/// and at least one.
pub fn frames(video: &Video) -> i64 {
    if video.duration() == Time::ZERO {
        1
    } else {
        video.rate.frame_at(video.duration() - Time(1)) + 1
    }
}

/// The size a video's frame shows at, no larger than `long` on its long
/// edge.
pub fn fitted(video: &Video, long: u32) -> (u32, u32) {
    let scale = (f64::from(long) / f64::from(video.width.max(video.height))).min(1.);
    (
        ((f64::from(video.width) * scale).round() as u32).max(2),
        ((f64::from(video.height) * scale).round() as u32).max(2),
    )
}

pub struct ProgramPictures {
    video: Video,
    sources: VideoSources<Box<Service>>,
}

impl ProgramPictures {
    pub fn open(root: PathBuf, video: Video) -> anyhow::Result<Self> {
        let service = Box::new(Service::open(&root)?);
        Ok(Self {
            video,
            sources: VideoSources::new(service, true, true),
        })
    }
}

impl Pictures for ProgramPictures {
    fn rate(&self) -> FrameRate {
        self.video.rate
    }

    fn last(&self) -> i64 {
        frames(&self.video) - 1
    }

    fn frame(&mut self, time: Time, size: u32, still: bool) -> anyhow::Result<(RgbaImage, i64)> {
        let index = self.video.rate.frame_at(time);
        let time = self.video.rate.time_of(index);
        // Rested frames are made as carefully as an export.
        let quality = if still {
            render::Quality::Final
        } else {
            render::Quality::Playback
        };
        let image = render::frame(
            &self.video,
            time,
            fitted(&self.video, size),
            &mut self.sources,
            quality,
        )?;
        Ok((image, index))
    }

    fn update(&mut self, update: Box<dyn std::any::Any + Send>) {
        if let Ok(video) = update.downcast::<Video>() {
            self.video = *video;
        }
    }
}

pub struct ProgramSound {
    video: Video,
    sources: VideoSources<Box<Service>>,
    position: Time,
}

impl ProgramSound {
    pub fn open(root: PathBuf, video: Video) -> anyhow::Result<Self> {
        let service = Box::new(Service::open(&root)?);
        Ok(Self {
            video,
            sources: VideoSources::new(service, false, false),
            position: Time::ZERO,
        })
    }
}

impl Sounds for ProgramSound {
    fn start(&mut self, from: Time, _track: usize) -> anyhow::Result<()> {
        self.position = from;
        Ok(())
    }

    fn read(&mut self, out: &mut [f32]) -> anyhow::Result<usize> {
        let end = self.video.duration();
        if self.position >= end {
            return Ok(0);
        }
        render::sound(&self.video, self.position, out, &mut self.sources)?;
        let frames = out.len() / 2;
        self.position = self.position + Time::from_samples(frames as i64, 48_000);
        Ok(frames)
    }

    fn update(&mut self, update: Box<dyn std::any::Any + Send>) {
        if let Ok(video) = update.downcast::<Video>() {
            self.video = *video;
        }
    }
}
