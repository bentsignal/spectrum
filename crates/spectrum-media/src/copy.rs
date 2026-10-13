//! Playback copies. Camera footage (4K, 10-bit, long groups of pictures)
//! decodes too slowly to scrub, play backward, or ramp speed smoothly, so
//! Spectrum makes a display-sized copy with short groups of pictures that
//! decodes thousands of frames a second. The copy has the same frames at the
//! same times; the original still shows whenever the picture is paused.
use std::path::Path;

use anyhow::{Context, Result, bail};
use ffmpeg_next::{self as ffmpeg, Rational, codec, encoder, ffi, format::Pixel};

use crate::{
    FrameReader, MediaInfo,
    convert::{Scaler, ScalerKey},
};

/// The long edge of a playback copy.
pub const COPY_LONG_EDGE: u32 = 1920;
/// Frames between keyframes: a seek decodes at most this many.
const GROUP: u32 = 8;

/// Whether footage plays better from a copy: larger than HD, or more than
/// 8 bits deep.
pub fn needs_playback_copy(info: &MediaInfo) -> bool {
    info.video
        .as_ref()
        .is_some_and(|video| video.width.max(video.height) > COPY_LONG_EDGE || video.bit_depth > 8)
}

/// Writes a playback copy of `source`'s picture to `destination` (a
/// Matroska file). `progress` hears the share done, from 0 to 1, and stops
/// the copy by returning false.
pub fn make_playback_copy(
    source: &Path,
    destination: &Path,
    progress: &mut dyn FnMut(f32) -> bool,
) -> Result<()> {
    crate::init();
    let mut reader = FrameReader::open(source)?;
    let rate = reader.rate();
    let frames = reader.frames();
    // Named for this attempt, so two processes making the same copy never
    // write into each other's file.
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temporary =
        destination.with_extension(format!("{}-{stamp}.partial.mkv", std::process::id()));
    let result = encode(&mut reader, &temporary, frames, rate, progress);
    match result {
        Ok(()) => {
            std::fs::rename(&temporary, destination)?;
            Ok(())
        }
        Err(error) => {
            let _ = std::fs::remove_file(&temporary);
            Err(error)
        }
    }
}

fn encode(
    reader: &mut FrameReader,
    path: &Path,
    frames: u64,
    rate: spectrum_time::FrameRate,
    progress: &mut dyn FnMut(f32) -> bool,
) -> Result<()> {
    let first = reader.picture(0)?;
    let (width, height) = even(crate::convert::fit(
        first.width(),
        first.height(),
        Some(COPY_LONG_EDGE),
    ));
    let (space, range) = (first.color_space(), first.color_range());
    let mut scaler = Scaler::new(ScalerKey {
        format: first.format(),
        width: first.width(),
        height: first.height(),
        out_width: width,
        out_height: height,
        space: space.into(),
        range: range.into(),
        output: Pixel::YUV420P,
    })?;
    let mut output = ffmpeg::format::output_as(path, "matroska")
        .with_context(|| format!("could not write {}", path.display()))?;
    let mpeg2 = encoder::find(codec::Id::MPEG2VIDEO).context("FFmpeg has no MPEG-2 encoder")?;
    let time_base = Rational::new(rate.denominator as i32, rate.numerator as i32);
    let global_header = output
        .format()
        .flags()
        .contains(ffmpeg::format::Flags::GLOBAL_HEADER);
    let mut context = codec::context::Context::new_with_codec(mpeg2)
        .encoder()
        .video()?;
    context.set_width(width);
    context.set_height(height);
    context.set_format(Pixel::YUV420P);
    context.set_time_base(time_base);
    context.set_frame_rate(Some(Rational::new(
        rate.numerator as i32,
        rate.denominator as i32,
    )));
    context.set_gop(GROUP);
    context.set_max_b_frames(0);
    context.set_colorspace(if space == ffmpeg::color::Space::Unspecified {
        ffmpeg::color::Space::BT709
    } else {
        space
    });
    context.set_color_range(ffmpeg::color::Range::MPEG);
    unsafe {
        let raw = context.as_mut_ptr();
        // A fixed quality: sharp enough to judge motion, small enough to
        // keep (about 8 GB an hour).
        (*raw).flags |= ffi::AV_CODEC_FLAG_QSCALE as i32;
        (*raw).global_quality = 4 * ffi::FF_QP2LAMBDA;
        (*raw).thread_count = 0;
        if global_header {
            (*raw).flags |= ffi::AV_CODEC_FLAG_GLOBAL_HEADER as i32;
        }
    }
    let mut encoder = context.open_as(mpeg2)?;
    {
        let mut stream = output.add_stream(mpeg2)?;
        stream.set_parameters(&encoder);
        stream.set_time_base(time_base);
        // Stored, so readers know the exact rate rather than estimating it
        // from rounded timestamps.
        let fps = Rational::new(rate.numerator as i32, rate.denominator as i32);
        stream.set_rate(fps);
        stream.set_avg_frame_rate(fps);
    }
    output.write_header()?;
    let stream_time_base = output.stream(0).context("no stream")?.time_base();
    let write = |encoder: &mut encoder::video::Encoder,
                 output: &mut ffmpeg::format::context::Output|
     -> Result<()> {
        let mut packet = ffmpeg::Packet::empty();
        while encoder.receive_packet(&mut packet).is_ok() {
            packet.set_stream(0);
            packet.rescale_ts(time_base, stream_time_base);
            packet.write_interleaved(output)?;
        }
        Ok(())
    };
    for index in 0..frames as i64 {
        let picture = reader.picture(index)?;
        let mut converted = scaler.run(picture)?;
        converted.set_pts(Some(index));
        encoder.send_frame(&converted)?;
        write(&mut encoder, &mut output)?;
        if index % 30 == 0 && !progress(index as f32 / frames as f32) {
            bail!("the playback copy was stopped");
        }
    }
    encoder.send_eof()?;
    write(&mut encoder, &mut output)?;
    output.write_trailer()?;
    progress(1.0);
    Ok(())
}

fn even((width, height): (u32, u32)) -> (u32, u32) {
    ((width & !1).max(2), (height & !1).max(2))
}
