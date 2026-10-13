//! Drawing a video's frame and mixing its sound at any time. Sources come
//! from providers, so exports, the command line, and the desktop's playback
//! (which reads playback copies) draw the same way: each video track's clip
//! is fitted into the frame and laid over the ones below at its opacity.
use anyhow::Result;
use image::RgbaImage;
use spectrum_time::Time;
use std::sync::Arc;

use crate::{Clip, TrackKind, Video};

/// How carefully frames between a source's frames are made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quality {
    /// Fast enough to play: motion interpolation blends instead.
    Playback,
    /// Paused frames and exports: everything as the clip asks.
    Final,
}

/// Pictures for clips. `source` is the exact source time in seconds (it
/// may fall between frames); `fit` is the box a picture must fit inside.
/// The provider returns it at that size (or smaller), in the byte order
/// the caller asked it for.
pub trait Pictures {
    fn picture(
        &mut self,
        clip: &Clip,
        source: f64,
        fit: (u32, u32),
        quality: Quality,
    ) -> Result<Option<Arc<RgbaImage>>>;
}

/// Sound for clips: `out.len() / 2` stereo 48 kHz sample frames of `clip`
/// from timeline time `from` onward, added into `out` at `gain`.
pub trait Sounds {
    fn mix(&mut self, clip: &Clip, from: Time, gain: f32, out: &mut [f32]) -> Result<()>;
}

/// The frame showing at `time`, `width` by `height`, over black.
pub fn frame(
    video: &Video,
    time: Time,
    (width, height): (u32, u32),
    pictures: &mut dyn Pictures,
    quality: Quality,
) -> Result<RgbaImage> {
    let clips = video.at(time, TrackKind::Video);
    // From the top down, until a picture fills the frame opaquely: nothing
    // under it shows, so nothing under it is decoded.
    let mut layers = Vec::with_capacity(clips.len());
    for clip in clips.iter().rev() {
        let picture =
            pictures.picture(clip, clip.source_seconds(time), (width, height), quality)?;
        let fills = picture
            .as_ref()
            .is_some_and(|p| clip.opacity >= 1. && covers(p, width, height));
        layers.push((*clip, picture));
        if fills {
            break;
        }
    }
    layers.reverse();
    if let [(clip, Some(picture))] = layers.as_slice()
        && clip.opacity >= 1.
        && picture.dimensions() == (width, height)
    {
        return Ok(picture.as_ref().clone());
    }
    let mut out = RgbaImage::from_pixel(width, height, image::Rgba([0, 0, 0, 255]));
    for (clip, picture) in &layers {
        if let Some(picture) = picture {
            lay(&mut out, picture, clip.opacity);
        }
    }
    Ok(out)
}

/// Whether a picture fills the whole frame without transparency.
fn covers(picture: &RgbaImage, width: u32, height: u32) -> bool {
    let (w, h) = picture.dimensions();
    (w + 1 >= width && h + 1 >= height) && picture.pixels().step_by(997).all(|p| p.0[3] == 255)
}

/// Lays `picture`, centered, over `out` at `opacity`.
fn lay(out: &mut RgbaImage, picture: &RgbaImage, opacity: f32) {
    let (width, height) = out.dimensions();
    let (w, h) = picture.dimensions();
    let x0 = (width.saturating_sub(w)) / 2;
    let y0 = (height.saturating_sub(h)) / 2;
    let opacity = (opacity.clamp(0., 1.) * 255.) as u32;
    for y in 0..h.min(height) {
        let row = &picture.as_raw()[(y * w * 4) as usize..((y + 1) * w * 4) as usize];
        let start = (((y0 + y) * width + x0) * 4) as usize;
        let target = &mut out.as_mut()[start..start + (w.min(width) * 4) as usize];
        let (pixels, sources) = (target.as_chunks_mut::<4>().0, row.as_chunks::<4>().0);
        for (dst, src) in pixels.iter_mut().zip(sources) {
            let alpha = u32::from(src[3]) * opacity / 255;
            if alpha == 255 {
                dst[..3].copy_from_slice(&src[..3]);
                continue;
            }
            for channel in 0..3 {
                let blended = (u32::from(src[channel]) * alpha
                    + u32::from(dst[channel]) * (255 - alpha))
                    / 255;
                dst[channel] = blended as u8;
            }
        }
    }
}

/// The video's sound for `out.len() / 2` stereo 48 kHz sample frames from
/// `time`, mixed from every audio track.
pub fn sound(video: &Video, time: Time, out: &mut [f32], sounds: &mut dyn Sounds) -> Result<()> {
    out.fill(0.);
    let frames = (out.len() / 2) as i64;
    let end = time + Time::from_samples(frames, 48_000);
    for track in video
        .tracks
        .iter()
        .filter(|t| t.kind == TrackKind::Audio && !t.muted)
    {
        for clip in track
            .clips
            .iter()
            .filter(|c| c.enabled && c.start < end && c.end() > time)
        {
            // The part of the window this clip plays in.
            let from = clip.start.max(time);
            let to = clip.end().min(end);
            let first = (from - time).samples(48_000).clamp(0, frames) as usize;
            let last = (to - time).samples(48_000).clamp(0, frames) as usize;
            if last <= first {
                continue;
            }
            let gain = 10f32.powf(clip.gain_db / 20.);
            sounds.mix(clip, from, gain, &mut out[first * 2..last * 2])?;
        }
    }
    for sample in out.iter_mut() {
        *sample = sample.clamp(-1., 1.);
    }
    Ok(())
}
