//! Decoding pictures at exact times. A [`FrameReader`] answers "which frame
//! shows at this time?" by seeking to the keyframe before it and decoding
//! forward, and decodes straight on when asked for later times, as playback
//! and forward scrubbing do.
use std::path::Path;

use anyhow::{Context, Result, bail};
use ffmpeg_next::{
    self as ffmpeg, Rational, codec, ffi, format::Pixel, format::stream::Disposition, media::Type,
    util::frame,
};
use image::RgbaImage;
use spectrum_time::{FrameRate, Time};

use crate::convert::{Scaler, ScalerKey, fit};

/// How far ahead a request may be before seeking beats decoding forward.
const DECODE_AHEAD: Time = Time(2 * spectrum_time::FLICKS_PER_SECOND);

/// A decoded picture, upright, as straight RGBA (or BGRA from a reader
/// made with [`FrameReader::bgra`]).
#[derive(Clone, Debug)]
pub struct Frame {
    pub image: RgbaImage,
    /// When the frame starts, from the start of the media.
    pub time: Time,
    /// The frame's number at the media's frame rate.
    pub index: i64,
}

struct Decoded {
    frame: frame::Video,
    time: Time,
    index: i64,
}

/// Decodes one file's picture.
pub struct FrameReader {
    input: ffmpeg::format::context::Input,
    stream: usize,
    decoder: ffmpeg::decoder::Video,
    time_base: Rational,
    /// The stream's first timestamp, which is the media's time zero.
    start: i64,
    rate: FrameRate,
    rotation: u16,
    duration: Time,
    frames: u64,
    scaler: Option<Scaler>,
    bgra: bool,
    /// The hardware decoding, if any.
    hardware: Option<&'static str>,
    /// The current frame copied from the GPU, once it is needed.
    in_memory: Option<frame::Video>,
    /// A frame came from the GPU, so the hardware decodes this stream.
    on_gpu: bool,
    current: Option<Decoded>,
    /// A frame decoded past the last answer, kept for the next request.
    ahead: Option<Decoded>,
    sent_eof: bool,
}

// The FFmpeg contexts are only used from the thread that owns the reader.
unsafe impl Send for FrameReader {}

impl FrameReader {
    pub fn open(path: &Path) -> Result<Self> {
        crate::init();
        let info = crate::probe(path)?;
        let video = info.video.context("the file has no picture")?;
        let input = ffmpeg::format::input(path)
            .with_context(|| format!("could not open {}", path.display()))?;
        let stream = input
            .streams()
            .find(|stream| {
                stream.parameters().medium() == Type::Video
                    && !stream.disposition().contains(Disposition::ATTACHED_PIC)
            })
            .context("the file has no picture")?;
        let index = stream.index();
        let time_base = stream.time_base();
        let start = if stream.start_time() == ffi::AV_NOPTS_VALUE {
            0
        } else {
            stream.start_time()
        };
        // MPEG-1 and -2 (playback copies among them) decode faster in
        // software than they download from the GPU, and VideoToolbox
        // refuses them on Apple silicon.
        let light = matches!(
            stream.parameters().id(),
            codec::Id::MPEG1VIDEO | codec::Id::MPEG2VIDEO
        );
        let open = |hardware: bool| -> Result<(ffmpeg::decoder::Video, Option<&'static str>)> {
            let mut context = codec::Context::from_parameters(stream.parameters())?;
            let mut used = None;
            unsafe {
                let raw = context.as_mut_ptr();
                (*raw).pkt_timebase = time_base.into();
                (*raw).thread_count = 0;
                (*raw).thread_type = ffi::FF_THREAD_FRAME | ffi::FF_THREAD_SLICE;
                if hardware && !light {
                    used = crate::hardware::attach(raw);
                }
            }
            let decoder = context.decoder().video()?;
            Ok((decoder, used))
        };
        // Hardware when it opens; software when it does not.
        let (decoder, hardware) = open(true)
            .or_else(|_| open(false))
            .with_context(|| format!("Spectrum cannot decode {} pictures", video.codec))?;
        Ok(Self {
            input,
            stream: index,
            decoder,
            time_base,
            start,
            rate: video.rate,
            rotation: video.rotation,
            duration: info.duration,
            frames: video.frames,
            scaler: None,
            bgra: false,
            hardware,
            in_memory: None,
            on_gpu: false,
            current: None,
            ahead: None,
            sent_eof: false,
        })
    }

    /// The hardware that decoded this file's frames so far, such as
    /// "VideoToolbox", or `None` for software.
    pub fn hardware(&self) -> Option<&'static str> {
        self.hardware.filter(|_| self.on_gpu)
    }

    /// Clockwise degrees that turn this file's frames upright.
    pub fn rotation(&self) -> u16 {
        self.rotation
    }

    /// Turns frames by `degrees` instead of the file's own rotation, as a
    /// playback copy of turned footage needs.
    pub fn with_rotation(mut self, degrees: u16) -> Self {
        self.rotation = degrees;
        self
    }

    /// The decoded picture showing at frame `index`, for re-encoding.
    pub(crate) fn picture(&mut self, index: i64) -> Result<&frame::Video> {
        let last = self.frames.saturating_sub(1) as i64;
        self.decode_to(index.clamp(0, last))?;
        let current = &self
            .current
            .as_ref()
            .context("the file has no frames")?
            .frame;
        crate::hardware::in_memory(current, &mut self.in_memory)
    }

    /// Frames in blue, green, red, alpha byte order from now on, as GPU
    /// uploads want them.
    pub fn bgra(mut self) -> Self {
        self.bgra = true;
        self
    }

    pub fn rate(&self) -> FrameRate {
        self.rate
    }

    /// How many frames the picture has.
    pub fn frames(&self) -> u64 {
        self.frames
    }

    pub fn duration(&self) -> Time {
        self.duration
    }

    /// The frame showing at `time`, no larger than `max_size` on its long
    /// edge (the full picture when `None`).
    pub fn frame_at(&mut self, time: Time, max_size: Option<u32>) -> Result<Frame> {
        let last = self.frames.saturating_sub(1) as i64;
        let wanted = self.rate.frame_at(time.max(Time::ZERO)).min(last);
        self.decode_to(wanted)?;
        let current = self.current.as_ref().context("the file has no frames")?;
        let (time, index) = (current.time, current.index);
        let image = self.convert(max_size)?;
        Ok(Frame { image, time, index })
    }

    /// The keyframe at or before `time`: a frame near it that decodes on
    /// its own, quickly, as posters need.
    pub fn keyframe_near(&mut self, time: Time, max_size: Option<u32>) -> Result<Frame> {
        self.seek(time)?;
        let first = self.decode_next()?.context("the file has no frames")?;
        let (time, index) = (first.time, first.index);
        self.set_current(first);
        let image = self.convert(max_size)?;
        Ok(Frame { image, time, index })
    }

    /// Frames up to and including frame `last`, oldest first: those from
    /// the keyframe before it, at most `max` of them. Playing backward
    /// steps through these, so each seek pays for a whole group of frames.
    pub fn frames_before(
        &mut self,
        last: i64,
        max: usize,
        max_size: Option<u32>,
    ) -> Result<Vec<Frame>> {
        let final_frame = self.frames.saturating_sub(1) as i64;
        let last = last.clamp(0, final_frame);
        let first = last - max.max(1) as i64 + 1;
        self.seek(self.rate.time_of(last))?;
        let mut frames = Vec::new();
        while let Some(next) = self.decode_next()? {
            if next.index > last {
                self.ahead = Some(next);
                break;
            }
            let index = next.index;
            self.set_current(next);
            if index >= first {
                let time = self.rate.time_of(index);
                frames.push(Frame {
                    image: self.convert(max_size)?,
                    time,
                    index,
                });
            }
        }
        Ok(frames)
    }

    /// Decodes until the current frame is the last one at or before frame
    /// `wanted`.
    fn decode_to(&mut self, wanted: i64) -> Result<()> {
        let ahead_of_wanted =
            |decoded: &Option<Decoded>| decoded.as_ref().is_some_and(|frame| frame.index > wanted);
        if let Some(current) = &self.current
            && current.index <= wanted
            && (ahead_of_wanted(&self.ahead) || (self.ahead.is_none() && self.sent_eof))
        {
            return Ok(());
        }
        let close_ahead = self.current.as_ref().is_some_and(|current| {
            current.index <= wanted && self.rate.time_of(wanted) - current.time <= DECODE_AHEAD
        });
        if !close_ahead {
            self.seek(self.rate.time_of(wanted))?;
        }
        loop {
            let next = match self.ahead.take() {
                Some(frame) => Some(frame),
                None => self.decode_next()?,
            };
            let Some(next) = next else {
                return Ok(());
            };
            if next.index <= wanted || self.current.is_none() {
                self.set_current(next);
            } else {
                self.ahead = Some(next);
                return Ok(());
            }
        }
    }

    fn set_current(&mut self, next: Decoded) {
        self.on_gpu |= crate::hardware::on_gpu(&next.frame);
        self.current = Some(next);
        self.in_memory = None;
    }

    /// Moves to the keyframe at or before `time`.
    fn seek(&mut self, time: Time) -> Result<()> {
        let target = self.start
            + (i128::from(time.0) * i128::from(self.time_base.denominator())
                / (i128::from(spectrum_time::FLICKS_PER_SECOND)
                    * i128::from(self.time_base.numerator().max(1)))) as i64;
        let result = unsafe {
            ffi::avformat_seek_file(
                self.input.as_mut_ptr(),
                self.stream as i32,
                i64::MIN,
                target,
                target,
                0,
            )
        };
        if result < 0 {
            // Some files only seek by byte or not at all: start over.
            let restarted = unsafe {
                ffi::avformat_seek_file(
                    self.input.as_mut_ptr(),
                    self.stream as i32,
                    i64::MIN,
                    i64::MIN,
                    i64::MIN,
                    ffi::AVSEEK_FLAG_ANY | ffi::AVSEEK_FLAG_BACKWARD,
                )
            };
            if restarted < 0 {
                bail!("could not seek in the file");
            }
        }
        self.decoder.flush();
        self.current = None;
        self.in_memory = None;
        self.ahead = None;
        self.sent_eof = false;
        Ok(())
    }

    /// The next frame in display order, or `None` at the end.
    fn decode_next(&mut self) -> Result<Option<Decoded>> {
        let mut decoded = frame::Video::empty();
        loop {
            match self.decoder.receive_frame(&mut decoded) {
                Ok(()) => {
                    let Some(stamp) = decoded.timestamp().or(decoded.pts()) else {
                        continue;
                    };
                    let time = Time::from_ratio(
                        (stamp - self.start) * i64::from(self.time_base.numerator()),
                        i64::from(self.time_base.denominator()),
                    );
                    // Containers round timestamps (Matroska to milliseconds),
                    // so a frame belongs to its nearest frame slot.
                    let index = self
                        .rate
                        .frame_at(time + Time(self.rate.frame_duration().0 / 2));
                    return Ok(Some(Decoded {
                        frame: decoded,
                        time,
                        index,
                    }));
                }
                Err(ffmpeg::Error::Eof) => return Ok(None),
                Err(ffmpeg::Error::Other { errno }) if errno == ffi::EAGAIN => {}
                Err(error) => return Err(error).context("could not decode a frame"),
            }
            if self.sent_eof {
                return Ok(None);
            }
            self.feed()?;
        }
    }

    /// Sends the decoder the stream's next packet, or the end of the stream.
    fn feed(&mut self) -> Result<()> {
        let mut packet = ffmpeg::Packet::empty();
        loop {
            match packet.read(&mut self.input) {
                Ok(()) if packet.stream() == self.stream => {
                    match self.decoder.send_packet(&packet) {
                        // Damaged data: skip it as players do.
                        Ok(()) | Err(ffmpeg::Error::InvalidData) => return Ok(()),
                        Err(error) => return Err(error).context("could not decode the picture"),
                    }
                }
                Ok(()) => {}
                Err(ffmpeg::Error::Eof) => {
                    self.decoder.send_eof()?;
                    self.sent_eof = true;
                    return Ok(());
                }
                Err(error) => return Err(error).context("could not read the file"),
            }
        }
    }

    /// The current frame as upright RGBA.
    fn convert(&mut self, max_size: Option<u32>) -> Result<RgbaImage> {
        let current = &self.current.as_ref().context("no frame decoded")?.frame;
        let decoded = crate::hardware::in_memory(current, &mut self.in_memory)?;
        let (width, height) = (decoded.width(), decoded.height());
        let (out_width, out_height) = fit(width, height, max_size);
        let key = ScalerKey {
            format: decoded.format(),
            width,
            height,
            out_width,
            out_height,
            space: decoded.color_space().into(),
            range: decoded.color_range().into(),
            output: if self.bgra { Pixel::BGRA } else { Pixel::RGBA },
        };
        if self.scaler.as_ref().is_none_or(|scaler| scaler.key != key) {
            self.scaler = Some(Scaler::new(key)?);
        }
        let image = self.scaler.as_mut().context("no scaler")?.image(decoded)?;
        Ok(match self.rotation {
            90 => image::imageops::rotate90(&image),
            180 => image::imageops::rotate180(&image),
            270 => image::imageops::rotate270(&image),
            _ => image,
        })
    }
}
