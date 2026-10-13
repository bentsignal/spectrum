//! Spectrum's videos: edits of video and audio tracks whose clips use
//! footage, audio, images, and canvases from the library, durable through
//! spectrum-document. The desktop timeline and `spectrum video` make the
//! same commands.

mod commands;
mod document;
mod edit;
pub mod render;
mod speed;

pub use commands::{Command, CommandOutput, Placement, VideoModel, Workspace, apply};
pub use document::{Clip, Interpolation, Source, Track, TrackKind, Video};
pub use edit::Edge;
pub use spectrum_time::{FrameRate, Time};
pub use speed::{Curve, MAX_PERCENT, RemapKey, Speed, VelocityKey};

#[cfg(test)]
mod tests;
