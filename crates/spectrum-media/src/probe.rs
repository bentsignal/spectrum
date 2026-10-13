//! What a media file holds: its picture, its sound tracks, and how long it
//! runs.
use std::{ffi::CStr, path::Path};

use anyhow::{Context, Result, bail};
use ffmpeg_next::{self as ffmpeg, ffi, format::stream::Disposition, media::Type};
use serde::{Deserialize, Serialize};
use spectrum_time::{FrameRate, Time};

/// A media file's streams, as Spectrum uses them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MediaInfo {
    /// The container, such as "mov,mp4,m4a,3gp,3g2,mj2" or "matroska,webm".
    pub container: String,
    pub duration: Time,
    /// The picture, for footage; audio files have none.
    pub video: Option<VideoInfo>,
    /// Every sound track, in file order. Screen recordings often keep a mix
    /// and separate sources on several tracks.
    pub audio: Vec<AudioInfo>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VideoInfo {
    pub codec: String,
    /// Upright size, after any rotation the file asks for.
    pub width: u32,
    pub height: u32,
    pub rate: FrameRate,
    /// Frames in the duration at `rate`.
    pub frames: u64,
    /// The decoded layout, such as "yuv422p10le".
    pub pixel_format: String,
    pub bit_depth: u8,
    /// Clockwise degrees (0, 90, 180, 270) that turn decoded frames upright.
    pub rotation: u16,
    /// The transfer curve, such as "bt709", "arib-std-b67" (HLG), or
    /// "smpte2084" (PQ).
    pub transfer: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AudioInfo {
    /// The track's place among the file's sound tracks, from 0.
    pub track: usize,
    pub codec: String,
    pub sample_rate: u32,
    pub channels: u16,
    pub title: Option<String>,
    pub language: Option<String>,
}

impl MediaInfo {
    pub fn is_footage(&self) -> bool {
        self.video.is_some()
    }
}

/// Reads a media file's streams without decoding it.
pub fn probe(path: &Path) -> Result<MediaInfo> {
    crate::init();
    let mut input = ffmpeg::format::input(path).with_context(|| {
        format!(
            "{} is not a video or audio file Spectrum can read",
            path.display()
        )
    })?;
    let container = input.format().name().to_string();
    let container_duration = (input.duration() > 0)
        .then(|| Time::from_ratio(input.duration(), i64::from(ffi::AV_TIME_BASE)));
    let mut video = None;
    let mut video_duration = None;
    let mut video_frames = 0;
    let mut video_stream = None;
    let mut audio = Vec::new();
    for stream in input.streams() {
        let parameters = stream.parameters();
        match parameters.medium() {
            Type::Video
                if video.is_none() && !stream.disposition().contains(Disposition::ATTACHED_PIC) =>
            {
                let time_base = stream.time_base();
                if stream.duration() > 0 {
                    video_duration = Some(Time::from_ratio(
                        stream.duration() * i64::from(time_base.numerator()),
                        i64::from(time_base.denominator()),
                    ));
                }
                video_frames = stream.frames().max(0) as u64;
                video_stream = Some((stream.index(), time_base, stream.start_time()));
                video = Some(video_info(&stream)?);
            }
            Type::Audio => {
                let (sample_rate, channels) = unsafe {
                    let raw = parameters.as_ptr();
                    (
                        (*raw).sample_rate.max(0) as u32,
                        (*raw).ch_layout.nb_channels.max(0) as u16,
                    )
                };
                if sample_rate == 0 || channels == 0 {
                    continue;
                }
                let metadata = stream.metadata();
                audio.push(AudioInfo {
                    track: audio.len(),
                    codec: parameters.id().name().to_string(),
                    sample_rate,
                    channels,
                    title: metadata.get("title").map(str::to_string),
                    language: metadata
                        .get("language")
                        .filter(|language| *language != "und")
                        .map(str::to_string),
                });
            }
            _ => {}
        }
    }
    if video_duration.is_none()
        && let Some((index, time_base, start)) = video_stream
    {
        video_duration = picture_end(&mut input, index, time_base, start, container_duration);
    }
    if video.is_none() && audio.is_empty() {
        bail!(
            "{} has no picture or sound Spectrum can read",
            path.display()
        );
    }
    let duration = video_duration
        .filter(|_| video.is_some())
        .or(container_duration)
        .context("the file does not say how long it runs")?;
    if let Some(video) = &mut video {
        // Counted frames when the file says; otherwise the duration in
        // frames, rounded, since containers round durations.
        video.frames = if video_frames > 0 {
            video_frames
        } else {
            (duration.seconds() * video.rate.fps()).round().max(1.0) as u64
        };
    }
    Ok(MediaInfo {
        container,
        duration,
        video,
        audio,
    })
}

/// When the picture ends, from its last packets, for containers (such as
/// Matroska) that give only the whole file's duration, which a sound track
/// can run past.
fn picture_end(
    input: &mut ffmpeg::format::context::Input,
    stream: usize,
    time_base: ffmpeg::Rational,
    start: i64,
    container: Option<Time>,
) -> Option<Time> {
    let start = if start == ffi::AV_NOPTS_VALUE {
        0
    } else {
        start
    };
    let near_end = container.map_or(0, |duration| {
        ((duration.seconds() - 3.0).max(0.0) * f64::from(ffi::AV_TIME_BASE)) as i64
    });
    input.seek(near_end, ..near_end).ok()?;
    let mut end = None;
    let mut packet = ffmpeg::Packet::empty();
    while packet.read(input).is_ok() {
        if packet.stream() != stream {
            continue;
        }
        if let Some(pts) = packet.pts() {
            let finish = pts + packet.duration().max(0);
            end = Some(end.map_or(finish, |end: i64| end.max(finish)));
        }
    }
    let end = end? - start;
    Some(Time::from_ratio(
        end * i64::from(time_base.numerator()),
        i64::from(time_base.denominator()),
    ))
}

fn video_info(stream: &ffmpeg::format::stream::Stream) -> Result<VideoInfo> {
    let parameters = stream.parameters();
    let rate = [stream.avg_frame_rate(), stream.rate()]
        .into_iter()
        .find(|rate| rate.numerator() > 0 && rate.denominator() > 0)
        .map(|rate| {
            FrameRate::new(rate.numerator() as u32, rate.denominator() as u32).standardized()
        })
        .context("the picture has no frame rate")?;
    let (width, height, format, transfer) = unsafe {
        let raw = parameters.as_ptr();
        (
            (*raw).width.max(0) as u32,
            (*raw).height.max(0) as u32,
            (*raw).format,
            (*raw).color_trc,
        )
    };
    if width == 0 || height == 0 {
        bail!("the picture has no size");
    }
    let rotation = rotation(&parameters);
    let (width, height) = if rotation % 180 == 90 {
        (height, width)
    } else {
        (width, height)
    };
    let (pixel_format, bit_depth) = unsafe {
        let format: ffi::AVPixelFormat = std::mem::transmute(format);
        let name = ffi::av_get_pix_fmt_name(format);
        let descriptor = ffi::av_pix_fmt_desc_get(format);
        (
            if name.is_null() {
                "unknown".to_string()
            } else {
                CStr::from_ptr(name).to_string_lossy().into_owned()
            },
            if descriptor.is_null() {
                8
            } else {
                (*descriptor).comp[0].depth as u8
            },
        )
    };
    let transfer = unsafe {
        let name = ffi::av_color_transfer_name(transfer);
        if name.is_null() {
            "unknown".to_string()
        } else {
            CStr::from_ptr(name).to_string_lossy().into_owned()
        }
    };
    Ok(VideoInfo {
        codec: parameters.id().name().to_string(),
        width,
        height,
        rate,
        frames: 0,
        pixel_format,
        bit_depth,
        rotation,
        transfer,
    })
}

/// Clockwise degrees that turn a stream's decoded frames upright, from its
/// display matrix.
pub(crate) fn rotation(parameters: &ffmpeg::codec::Parameters) -> u16 {
    unsafe {
        let raw = parameters.as_ptr();
        let side = ffi::av_packet_side_data_get(
            (*raw).coded_side_data,
            (*raw).nb_coded_side_data,
            ffi::AVPacketSideDataType::AV_PKT_DATA_DISPLAYMATRIX,
        );
        if side.is_null() || (*side).size < 36 {
            return 0;
        }
        // The matrix gives counterclockwise degrees.
        let degrees = ffi::av_display_rotation_get((*side).data as *const i32);
        if !degrees.is_finite() {
            return 0;
        }
        let clockwise = (-degrees).rem_euclid(360.0);
        ((clockwise / 90.0).round() as u16 % 4) * 90
    }
}
