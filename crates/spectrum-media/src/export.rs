//! Writing videos: frames and 48 kHz stereo sound in, an MP4 or MOV file
//! out. H.264 and HEVC use the best encoder this build has (x264 or x265
//! where FFmpeg includes them, the platform's hardware encoder otherwise);
//! ProRes 422 HQ is FFmpeg's own.
use std::path::Path;

use anyhow::{Context, Result, bail};
use ffmpeg_next::{
    self as ffmpeg, ChannelLayout, Dictionary, Rational, codec, encoder, ffi, format::Pixel,
    format::Sample, format::sample::Type as SampleType, util::frame,
};
use image::RgbaImage;
use serde::{Deserialize, Serialize};
use spectrum_time::FrameRate;

use crate::convert::{Scaler, ScalerKey};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VideoCodec {
    /// H.264 in MP4: plays everywhere.
    #[default]
    H264,
    /// HEVC (H.265) in MP4: half the size at the same quality.
    Hevc,
    /// Apple ProRes 422 HQ in MOV: for further editing.
    ProRes,
}

impl VideoCodec {
    pub fn extension(self) -> &'static str {
        match self {
            VideoCodec::ProRes => "mov",
            _ => "mp4",
        }
    }

    /// Encoders to try, best first.
    fn encoders(self) -> &'static [&'static str] {
        match self {
            VideoCodec::H264 => &[
                "libx264",
                "h264_videotoolbox",
                "h264_nvenc",
                "h264_qsv",
                "h264_amf",
                "h264_mf",
                "libopenh264",
            ],
            VideoCodec::Hevc => &[
                "libx265",
                "hevc_videotoolbox",
                "hevc_nvenc",
                "hevc_qsv",
                "hevc_amf",
                "hevc_mf",
            ],
            VideoCodec::ProRes => &["prores_ks"],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExportSettings {
    pub codec: VideoCodec,
    pub width: u32,
    pub height: u32,
    pub rate: FrameRate,
    /// Bits a second for H.264 and HEVC; picked from the size and rate when
    /// `None`.
    #[serde(default)]
    pub bitrate: Option<u64>,
    #[serde(default = "yes")]
    pub sound: bool,
}

fn yes() -> bool {
    true
}

impl ExportSettings {
    /// About 0.12 bits a pixel each frame: 15 Mb/s for 1080p at 60 fps,
    /// 60 Mb/s for 4K at 60; HEVC needs about two thirds as much.
    fn bitrate(&self) -> u64 {
        self.bitrate.unwrap_or_else(|| {
            let pixels = f64::from(self.width) * f64::from(self.height) * self.rate.fps();
            let factor = if self.codec == VideoCodec::Hevc {
                0.08
            } else {
                0.12
            };
            (pixels * factor) as u64
        })
    }
}

/// An export being written: push every frame in order, the sound along
/// with them, then finish.
pub struct Writer {
    output: ffmpeg::format::context::Output,
    video: encoder::video::Encoder,
    video_base: Rational,
    video_stream: Rational,
    scaler: Option<Scaler>,
    format: Pixel,
    frames: i64,
    sound: Option<Sound>,
    /// The encoder chosen, such as "libx264" or "h264_videotoolbox".
    pub encoder: &'static str,
}

struct Sound {
    encoder: encoder::audio::Encoder,
    stream: Rational,
    pending: Vec<f32>,
    samples: i64,
    frame_size: usize,
}

// The FFmpeg contexts are only used from the thread that owns the writer.
unsafe impl Send for Writer {}

impl Writer {
    pub fn create(path: &Path, settings: ExportSettings) -> Result<Self> {
        crate::init();
        if settings.width % 2 == 1 || settings.height % 2 == 1 {
            bail!("exports need an even width and height");
        }
        let container = match settings.codec {
            VideoCodec::ProRes => "mov",
            _ => "mp4",
        };
        let mut output = ffmpeg::format::output_as(path, container)
            .with_context(|| format!("could not write {}", path.display()))?;
        let global = output
            .format()
            .flags()
            .contains(ffmpeg::format::Flags::GLOBAL_HEADER);
        let (video, name, format) = open_video(&settings, global)?;
        let video_base = Rational::new(
            settings.rate.denominator as i32,
            settings.rate.numerator as i32,
        );
        {
            let mut stream = output.add_stream(encoder::find_by_name(name))?;
            stream.set_parameters(&video);
            stream.set_time_base(video_base);
            let fps = Rational::new(
                settings.rate.numerator as i32,
                settings.rate.denominator as i32,
            );
            stream.set_rate(fps);
            stream.set_avg_frame_rate(fps);
        }
        let sound = if settings.sound {
            let encoder = open_sound(global)?;
            let mut stream = output.add_stream(encoder::find(codec::Id::AAC))?;
            stream.set_parameters(&encoder);
            stream.set_time_base(Rational::new(1, 48_000));
            let frame_size = (encoder.frame_size() as usize).max(1024);
            Some(Sound {
                encoder,
                stream: Rational::new(1, 48_000),
                pending: Vec::new(),
                samples: 0,
                frame_size,
            })
        } else {
            None
        };
        let mut options = Dictionary::new();
        options.set("movflags", "+faststart");
        output.write_header_with(options)?;
        let video_stream = output.stream(0).context("no picture stream")?.time_base();
        let mut writer = Self {
            output,
            video,
            video_base,
            video_stream,
            scaler: None,
            format,
            frames: 0,
            sound,
            encoder: name,
        };
        if let Some(sound) = &mut writer.sound {
            sound.stream = writer
                .output
                .stream(1)
                .context("no sound stream")?
                .time_base();
        }
        Ok(writer)
    }

    /// Adds the next frame, straight RGBA at the export's size.
    pub fn push_frame(&mut self, image: &RgbaImage) -> Result<()> {
        let (width, height) = image.dimensions();
        let mut rgba = frame::Video::new(Pixel::RGBA, width, height);
        let stride = rgba.stride(0);
        let data = rgba.data_mut(0);
        for (y, row) in image.as_raw().chunks_exact(width as usize * 4).enumerate() {
            data[y * stride..y * stride + row.len()].copy_from_slice(row);
        }
        let key = ScalerKey {
            format: Pixel::RGBA,
            width,
            height,
            out_width: self.video.width(),
            out_height: self.video.height(),
            space: ffi::AVColorSpace::AVCOL_SPC_RGB,
            range: ffi::AVColorRange::AVCOL_RANGE_JPEG,
            output: self.format,
        };
        if self.scaler.as_ref().is_none_or(|s| s.key != key) {
            self.scaler = Some(Scaler::new(key)?);
        }
        let mut converted = self.scaler.as_mut().context("no scaler")?.run(&rgba)?;
        converted.set_pts(Some(self.frames));
        self.frames += 1;
        self.video.send_frame(&converted)?;
        self.drain_video()
    }

    /// Adds interleaved 48 kHz stereo samples.
    pub fn push_sound(&mut self, samples: &[f32]) -> Result<()> {
        let Some(sound) = &mut self.sound else {
            return Ok(());
        };
        sound.pending.extend_from_slice(samples);
        while sound.pending.len() >= sound.frame_size * 2 {
            let chunk: Vec<f32> = sound.pending.drain(..sound.frame_size * 2).collect();
            send_sound(sound, &chunk)?;
            drain_sound(sound, &mut self.output)?;
        }
        Ok(())
    }

    /// Flushes the encoders and closes the file.
    pub fn finish(mut self) -> Result<()> {
        self.video.send_eof()?;
        self.drain_video()?;
        if let Some(sound) = &mut self.sound {
            if !sound.pending.is_empty() {
                let mut last = std::mem::take(&mut sound.pending);
                last.resize(sound.frame_size * 2, 0.);
                send_sound(sound, &last)?;
            }
            sound.encoder.send_eof()?;
            drain_sound(sound, &mut self.output)?;
        }
        self.output.write_trailer()?;
        Ok(())
    }

    fn drain_video(&mut self) -> Result<()> {
        let mut packet = ffmpeg::Packet::empty();
        while self.video.receive_packet(&mut packet).is_ok() {
            packet.set_stream(0);
            packet.rescale_ts(self.video_base, self.video_stream);
            packet.write_interleaved(&mut self.output)?;
        }
        Ok(())
    }
}

fn open_video(
    settings: &ExportSettings,
    global: bool,
) -> Result<(encoder::video::Encoder, &'static str, Pixel)> {
    let mut failures = Vec::new();
    for &name in settings.codec.encoders() {
        let Some(found) = encoder::find_by_name(name) else {
            continue;
        };
        let format = match settings.codec {
            VideoCodec::ProRes => Pixel::YUV422P10LE,
            _ if name.contains("videotoolbox") || name.ends_with("_qsv") => Pixel::NV12,
            _ => Pixel::YUV420P,
        };
        let mut context = codec::context::Context::new_with_codec(found)
            .encoder()
            .video()?;
        context.set_width(settings.width);
        context.set_height(settings.height);
        context.set_format(format);
        context.set_time_base(Rational::new(
            settings.rate.denominator as i32,
            settings.rate.numerator as i32,
        ));
        context.set_frame_rate(Some(Rational::new(
            settings.rate.numerator as i32,
            settings.rate.denominator as i32,
        )));
        context.set_colorspace(ffmpeg::color::Space::BT709);
        context.set_color_range(ffmpeg::color::Range::MPEG);
        context.set_gop((settings.rate.fps() * 2.).round() as u32);
        let mut options = Dictionary::new();
        match (settings.codec, name) {
            (VideoCodec::ProRes, _) => options.set("profile", "3"),
            (_, "libx264" | "libx265") => {
                options.set("crf", if name == "libx264" { "18" } else { "20" });
                options.set("preset", "medium");
            }
            _ => {
                context.set_bit_rate(settings.bitrate() as usize);
                context.set_max_bit_rate((settings.bitrate() * 3 / 2) as usize);
            }
        }
        unsafe {
            let raw = context.as_mut_ptr();
            (*raw).thread_count = 0;
            (*raw).color_primaries = ffi::AVColorPrimaries::AVCOL_PRI_BT709;
            (*raw).color_trc = ffi::AVColorTransferCharacteristic::AVCOL_TRC_BT709;
            if global {
                (*raw).flags |= ffi::AV_CODEC_FLAG_GLOBAL_HEADER as i32;
            }
        }
        match context.open_as_with(found, options) {
            Ok(opened) => return Ok((opened, name, format)),
            Err(error) => failures.push(format!("{name}: {error}")),
        }
    }
    bail!(
        "this computer has no {:?} encoder Spectrum can use ({})",
        settings.codec,
        failures.join("; ")
    )
}

fn open_sound(global: bool) -> Result<encoder::audio::Encoder> {
    let aac = encoder::find(codec::Id::AAC).context("FFmpeg has no AAC encoder")?;
    let mut context = codec::context::Context::new_with_codec(aac)
        .encoder()
        .audio()?;
    context.set_rate(48_000);
    context.set_channel_layout(ChannelLayout::STEREO);
    context.set_format(Sample::F32(SampleType::Planar));
    context.set_bit_rate(320_000);
    context.set_time_base(Rational::new(1, 48_000));
    if global {
        unsafe {
            (*context.as_mut_ptr()).flags |= ffi::AV_CODEC_FLAG_GLOBAL_HEADER as i32;
        }
    }
    Ok(context.open_as(aac)?)
}

fn send_sound(sound: &mut Sound, interleaved: &[f32]) -> Result<()> {
    let count = interleaved.len() / 2;
    let mut planar = frame::Audio::new(
        Sample::F32(SampleType::Planar),
        count,
        ChannelLayout::STEREO,
    );
    planar.set_rate(48_000);
    for channel in 0..2 {
        let plane = planar.plane_mut::<f32>(channel);
        for (index, sample) in plane.iter_mut().enumerate().take(count) {
            *sample = interleaved[index * 2 + channel];
        }
    }
    planar.set_pts(Some(sound.samples));
    sound.samples += count as i64;
    sound.encoder.send_frame(&planar)?;
    Ok(())
}

fn drain_sound(sound: &mut Sound, output: &mut ffmpeg::format::context::Output) -> Result<()> {
    let mut packet = ffmpeg::Packet::empty();
    while sound.encoder.receive_packet(&mut packet).is_ok() {
        packet.set_stream(1);
        packet.rescale_ts(Rational::new(1, 48_000), sound.stream);
        packet.write_interleaved(output)?;
    }
    Ok(())
}
