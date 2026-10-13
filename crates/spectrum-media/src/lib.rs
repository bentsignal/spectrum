//! Spectrum's footage and audio. Files are read through FFmpeg, so every
//! common container and codec opens: [`probe`] reads what a file holds,
//! [`FrameReader`] decodes its pictures at exact times, and [`AudioReader`]
//! decodes its sound as 48 kHz stereo. [`Media`] is the library document
//! for an imported file; the file itself lives in the library's media store.

mod audio;
mod convert;
mod copy;
mod document;
mod export;
mod hardware;
mod probe;
mod video;

pub use audio::{AudioReader, CHANNELS, SAMPLE_RATE, waveform, waveform_with};
pub use copy::{COPY_LONG_EDGE, make_playback_copy, needs_playback_copy};
pub use document::{Command, CommandOutput, Media, MediaModel, Workspace};
pub use export::{ExportSettings, VideoCodec, Writer};
pub use probe::{AudioInfo, MediaInfo, VideoInfo, probe};
pub use spectrum_time::{FrameRate, Time};
pub use video::{Frame, FrameReader};

use std::sync::Once;

/// Readies FFmpeg once per process and keeps its console logging to errors.
pub(crate) fn init() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        let _ = ffmpeg_next::init();
        ffmpeg_next::util::log::set_level(ffmpeg_next::util::log::Level::Error);
    });
}

/// File extensions Spectrum offers to import as footage or audio. Probing
/// decides; this only filters file pickers and folders.
pub fn is_media_file(path: &std::path::Path) -> bool {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    matches!(
        extension.as_str(),
        "mp4"
            | "m4v"
            | "mov"
            | "mkv"
            | "webm"
            | "avi"
            | "mts"
            | "m2ts"
            | "mxf"
            | "mpg"
            | "mpeg"
            | "ts"
            | "flv"
            | "wmv"
            | "3gp"
            | "mp3"
            | "wav"
            | "flac"
            | "aac"
            | "m4a"
            | "ogg"
            | "oga"
            | "opus"
            | "aif"
            | "aiff"
            | "wma"
            | "alac"
    )
}

#[cfg(test)]
mod tests;
