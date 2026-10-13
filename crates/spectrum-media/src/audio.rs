//! Decoding sound as 48 kHz stereo floats, whatever the file holds: other
//! rates are resampled and surround is mixed down.
use std::{collections::VecDeque, path::Path, ptr};

use anyhow::{Context, Result, bail};
use ffmpeg_next::{self as ffmpeg, Rational, codec, ffi, media::Type, util::frame};
use spectrum_time::Time;

pub const SAMPLE_RATE: u32 = 48_000;
pub const CHANNELS: usize = 2;

/// Decodes one sound track of a file.
pub struct AudioReader {
    input: ffmpeg::format::context::Input,
    stream: usize,
    decoder: ffmpeg::decoder::Audio,
    time_base: Rational,
    start: i64,
    resampler: Resampler,
    /// Decoded, resampled samples not read yet, interleaved.
    buffer: VecDeque<f32>,
    /// The sample frame `buffer` starts at, once known after a seek.
    position: Option<i64>,
    /// The sample frame reading should start from after a seek.
    target: i64,
    sent_eof: bool,
    finished: bool,
}

// The FFmpeg contexts are only used from the thread that owns the reader.
unsafe impl Send for AudioReader {}

impl AudioReader {
    /// Opens sound track `track` (0 is the first) of a file.
    pub fn open(path: &Path, track: usize) -> Result<Self> {
        crate::init();
        let input = ffmpeg::format::input(path)
            .with_context(|| format!("could not open {}", path.display()))?;
        let stream = input
            .streams()
            .filter(|stream| stream.parameters().medium() == Type::Audio)
            .nth(track)
            .with_context(|| format!("the file has no sound track {}", track + 1))?;
        let index = stream.index();
        let time_base = stream.time_base();
        let start = if stream.start_time() == ffi::AV_NOPTS_VALUE {
            0
        } else {
            stream.start_time()
        };
        let mut context = codec::Context::from_parameters(stream.parameters())?;
        unsafe {
            (*context.as_mut_ptr()).pkt_timebase = time_base.into();
        }
        let decoder = context
            .decoder()
            .audio()
            .context("Spectrum cannot decode this sound")?;
        Ok(Self {
            input,
            stream: index,
            decoder,
            time_base,
            start,
            resampler: Resampler::default(),
            buffer: VecDeque::new(),
            position: Some(0),
            target: 0,
            sent_eof: false,
            finished: false,
        })
    }

    /// Continues reading from `time`.
    pub fn seek(&mut self, time: Time) -> Result<()> {
        let time = time.max(Time::ZERO);
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
            bail!("could not seek in the sound");
        }
        self.decoder.flush();
        self.resampler = Resampler::default();
        self.buffer.clear();
        self.position = None;
        self.target = time.samples(SAMPLE_RATE);
        self.sent_eof = false;
        self.finished = false;
        Ok(())
    }

    /// Fills `out` with interleaved stereo samples, returning how many sample
    /// frames were written; fewer than asked means the sound ended.
    pub fn read(&mut self, out: &mut [f32]) -> Result<usize> {
        let wanted = out.len() / CHANNELS * CHANNELS;
        while self.buffer.len() < wanted && !self.finished {
            self.decode_more()?;
        }
        let count = wanted.min(self.buffer.len());
        for (slot, sample) in out.iter_mut().zip(self.buffer.drain(..count)) {
            *slot = sample;
        }
        if let Some(position) = &mut self.position {
            *position += (count / CHANNELS) as i64;
        }
        Ok(count / CHANNELS)
    }

    /// Decodes one more frame into the buffer, lining the first one after a
    /// seek up with the requested time.
    fn decode_more(&mut self) -> Result<()> {
        let mut decoded = frame::Audio::empty();
        loop {
            match self.decoder.receive_frame(&mut decoded) {
                Ok(()) => break,
                Err(ffmpeg::Error::Eof) => {
                    self.finished = true;
                    return Ok(());
                }
                Err(ffmpeg::Error::Other { errno }) if errno == ffi::EAGAIN => {}
                Err(error) => return Err(error).context("could not decode the sound"),
            }
            if self.sent_eof {
                self.finished = true;
                return Ok(());
            }
            self.feed()?;
        }
        let samples = self.resampler.convert(&decoded)?;
        if self.position.is_none() {
            let stamp = decoded.timestamp().or(decoded.pts()).unwrap_or(self.start);
            let time = Time::from_ratio(
                (stamp - self.start) * i64::from(self.time_base.numerator()),
                i64::from(self.time_base.denominator()),
            );
            let first = time.samples(SAMPLE_RATE);
            let skip = self.target - first;
            if skip > 0 {
                let skip = (skip as usize * CHANNELS).min(samples.len());
                self.buffer.extend(&samples[skip..]);
                if skip == samples.len() {
                    // Still before the target: keep looking.
                    return Ok(());
                }
            } else {
                // The sound starts after the target: silence until it does.
                self.buffer
                    .extend(std::iter::repeat_n(0.0, (-skip) as usize * CHANNELS));
                self.buffer.extend(&samples);
            }
            self.position = Some(self.target);
            return Ok(());
        }
        self.buffer.extend(&samples);
        Ok(())
    }

    fn feed(&mut self) -> Result<()> {
        let mut packet = ffmpeg::Packet::empty();
        loop {
            match packet.read(&mut self.input) {
                Ok(()) if packet.stream() == self.stream => {
                    match self.decoder.send_packet(&packet) {
                        Ok(()) | Err(ffmpeg::Error::InvalidData) => return Ok(()),
                        Err(error) => return Err(error).context("could not decode the sound"),
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
}

/// Converts decoded frames to 48 kHz interleaved stereo floats, rebuilding
/// itself when the input's format changes.
#[derive(Default)]
struct Resampler {
    context: Option<(Input, *mut ffi::SwrContext)>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Input {
    format: i32,
    rate: i32,
    channels: i32,
    order: u32,
    mask: u64,
}

impl Resampler {
    fn convert(&mut self, decoded: &frame::Audio) -> Result<Vec<f32>> {
        let raw = unsafe { &*decoded.as_ptr() };
        let input = Input {
            format: raw.format,
            rate: raw.sample_rate,
            channels: raw.ch_layout.nb_channels,
            order: raw.ch_layout.order as u32,
            mask: unsafe { raw.ch_layout.u.mask },
        };
        if self.context.is_none_or(|(current, _)| current != input) {
            self.release();
            self.context = Some((input, open_resampler(raw)?));
        }
        let (_, context) = self.context.context("no resampler")?;
        unsafe {
            let capacity = ffi::swr_get_out_samples(context, raw.nb_samples).max(0) as usize;
            let mut out = vec![0f32; capacity * CHANNELS];
            let mut out_pointer = out.as_mut_ptr() as *mut u8;
            let converted = ffi::swr_convert(
                context,
                // `*mut` or `*const` by FFmpeg version.
                &raw mut out_pointer as _,
                capacity as i32,
                raw.extended_data as *mut *const u8,
                raw.nb_samples,
            );
            if converted < 0 {
                bail!("could not convert the sound");
            }
            out.truncate(converted as usize * CHANNELS);
            Ok(out)
        }
    }

    fn release(&mut self) {
        if let Some((_, mut context)) = self.context.take() {
            unsafe { ffi::swr_free(&mut context) };
        }
    }
}

impl Drop for Resampler {
    fn drop(&mut self) {
        self.release();
    }
}

fn open_resampler(raw: &ffi::AVFrame) -> Result<*mut ffi::SwrContext> {
    unsafe {
        let mut in_layout: ffi::AVChannelLayout = std::mem::zeroed();
        if raw.ch_layout.order == ffi::AVChannelOrder::AV_CHANNEL_ORDER_UNSPEC {
            ffi::av_channel_layout_default(&mut in_layout, raw.ch_layout.nb_channels);
        } else if ffi::av_channel_layout_copy(&mut in_layout, &raw.ch_layout) < 0 {
            bail!("could not read the sound's channels");
        }
        let mut out_layout: ffi::AVChannelLayout = std::mem::zeroed();
        ffi::av_channel_layout_default(&mut out_layout, CHANNELS as i32);
        let mut context = ptr::null_mut();
        let format: ffi::AVSampleFormat = std::mem::transmute(raw.format);
        let result = ffi::swr_alloc_set_opts2(
            &mut context,
            &out_layout,
            ffi::AVSampleFormat::AV_SAMPLE_FMT_FLT,
            SAMPLE_RATE as i32,
            &in_layout,
            format,
            raw.sample_rate,
            0,
            ptr::null_mut(),
        );
        if in_layout.nb_channels == 1 && !context.is_null() {
            // Mono plays at full level on both sides, not mixed down 3 dB
            // as a surround center would be.
            let unity = [1.0f64, 1.0];
            ffi::swr_set_matrix(context, unity.as_ptr(), 1);
        }
        ffi::av_channel_layout_uninit(&mut in_layout);
        ffi::av_channel_layout_uninit(&mut out_layout);
        if result < 0 || context.is_null() || ffi::swr_init(context) < 0 {
            ffi::swr_free(&mut context);
            bail!("could not convert the sound to 48 kHz stereo");
        }
        Ok(context)
    }
}

/// The loudest sample in each of `buckets` equal spans of a sound track,
/// from 0 to 1, for drawing its waveform.
pub fn waveform(path: &Path, track: usize, duration: Time, buckets: usize) -> Result<Vec<f32>> {
    waveform_with(path, track, duration, buckets, &mut || {})
}

/// The same, calling `between` before each chunk it reads, so background
/// work can wait there while something more pressing runs.
pub fn waveform_with(
    path: &Path,
    track: usize,
    duration: Time,
    buckets: usize,
    between: &mut dyn FnMut(),
) -> Result<Vec<f32>> {
    let mut reader = AudioReader::open(path, track)?;
    let total = duration.samples(SAMPLE_RATE).max(1) as usize;
    let mut peaks = vec![0f32; buckets.max(1)];
    let mut chunk = vec![0f32; 8192 * CHANNELS];
    let mut position = 0usize;
    loop {
        between();
        let frames = reader.read(&mut chunk)?;
        for frame in chunk[..frames * CHANNELS].as_chunks::<CHANNELS>().0 {
            let bucket = (position * peaks.len() / total).min(peaks.len() - 1);
            let level = frame
                .iter()
                .fold(0f32, |peak, sample| peak.max(sample.abs()));
            peaks[bucket] = peaks[bucket].max(level.min(1.0));
            position += 1;
        }
        if frames * CHANNELS < chunk.len() {
            return Ok(peaks);
        }
    }
}
