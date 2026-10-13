//! The player's picture thread. It owns where pictures come from (a file's
//! decoder, or a video's frames drawn from its clips): shown a time, it
//! makes that frame; playing, it works ahead in the direction of play and
//! hands frames over as they are ready. Only the newest request counts, so
//! a fast scrub never queues stale work.
use gpui::RenderImage;
use image::RgbaImage;
use spectrum_media::{FrameRate, FrameReader, Time};
use std::{path::PathBuf, sync::Arc, time::Duration};

/// Where the picture thread gets frames.
pub trait Pictures: Send {
    fn rate(&self) -> FrameRate;
    /// The last frame's number.
    fn last(&self) -> i64;
    /// The frame showing at `time`, no larger than `size` on its long edge,
    /// in blue, green, red, alpha order, and its number.
    /// `still` asks for a paused frame, which may take longer to make well.
    fn frame(&mut self, time: Time, size: u32, still: bool) -> anyhow::Result<(RgbaImage, i64)>;
    /// Frames up to `last`, oldest first, as many as is cheap: playing
    /// backward steps through them.
    fn frames_before(&mut self, last: i64, size: u32) -> anyhow::Result<Vec<(RgbaImage, i64)>> {
        let rate = self.rate();
        Ok(vec![self.frame(rate.time_of(last), size, false)?])
    }
    /// The hardware decoding, if any.
    fn hardware(&self) -> Option<&'static str> {
        None
    }
    /// Takes a newer version of what it shows, such as an edited video.
    fn update(&mut self, _: Box<dyn std::any::Any + Send>) {}
}

impl Pictures for FrameReader {
    fn rate(&self) -> FrameRate {
        FrameReader::rate(self)
    }

    fn last(&self) -> i64 {
        self.frames().max(1) as i64 - 1
    }

    fn frame(&mut self, time: Time, size: u32, _still: bool) -> anyhow::Result<(RgbaImage, i64)> {
        let frame = self.frame_at(time, Some(size))?;
        Ok((frame.image, frame.index))
    }

    fn frames_before(&mut self, last: i64, size: u32) -> anyhow::Result<Vec<(RgbaImage, i64)>> {
        Ok(
            FrameReader::frames_before(self, last, REVERSE_CHUNK, Some(size))?
                .into_iter()
                .map(|frame| (frame.image, frame.index))
                .collect(),
        )
    }

    fn hardware(&self) -> Option<&'static str> {
        FrameReader::hardware(self)
    }
}

/// Frames decoded ahead of playback.
pub const AHEAD: usize = 6;
/// Most frames decoded at once when playing backward: forward from the
/// keyframe before, then handed over last first.
const REVERSE_CHUNK: usize = 90;

pub enum Request {
    /// Show the frame at a time, no larger than `size` on its long edge.
    Show {
        time: Time,
        size: u32,
        generation: u64,
        /// Made as carefully as a paused frame deserves; otherwise quickly,
        /// as a scrub needs.
        fine: bool,
    },
    /// Decode ahead from `from`, `step` frames at a time (negative plays
    /// backward).
    Play {
        from: Time,
        step: i64,
        size: u32,
        generation: u64,
    },
    /// A newer version of what the pictures show; nothing is drawn until
    /// the next request.
    Update(Box<dyn std::any::Any + Send>),
}

pub struct Decoded {
    pub image: Arc<RenderImage>,
    pub index: i64,
    pub generation: u64,
    /// Answers a `Show`, rather than a frame of playback.
    pub still: bool,
    /// Made carefully, after a quick one of the same request.
    pub fine: bool,
    /// The hardware that decoded it, or `None` for software.
    pub hardware: Option<&'static str>,
}

/// What the thread hands over: a frame, or why the file could not open.
pub type Delivery = Result<Decoded, String>;

/// Starts the thread for `file`, turning its frames by `rotation` instead
/// of the file's own when given; it ends when `requests` is dropped. The
/// file opens on the thread, so the window never waits for it.
pub fn start(
    file: PathBuf,
    rotation: Option<u16>,
) -> anyhow::Result<(flume::Sender<Request>, flume::Receiver<Delivery>)> {
    start_with(move || {
        let reader = FrameReader::open(&file)?.bgra();
        Ok(Box::new(match rotation {
            Some(rotation) => reader.with_rotation(rotation),
            None => reader,
        }) as Box<dyn Pictures>)
    })
}

/// Starts the thread with pictures made on it by `open`.
pub fn start_with(
    open: impl FnOnce() -> anyhow::Result<Box<dyn Pictures>> + Send + 'static,
) -> anyhow::Result<(flume::Sender<Request>, flume::Receiver<Delivery>)> {
    let (requests, incoming) = flume::unbounded();
    let (frames, received) = flume::bounded(AHEAD);
    std::thread::Builder::new()
        .name("spectrum-frames".into())
        .spawn(move || match open() {
            Ok(pictures) => run(pictures, incoming, frames),
            Err(error) => {
                let _ = frames.send(Err(format!("{error:#}")));
            }
        })?;
    Ok((requests, received))
}

struct Playing {
    next: i64,
    step: i64,
    size: u32,
    generation: u64,
    /// Backward play's decoded chunk, handed over from the end.
    chunk: Vec<Decoded>,
}

fn run(
    mut reader: Box<dyn Pictures>,
    requests: flume::Receiver<Request>,
    frames: flume::Sender<Delivery>,
) {
    let mut playing: Option<Playing> = None;
    loop {
        // Take the newest request; block only while idle.
        // Updates all apply, in order; only the newest show or play counts.
        let mut newest = None;
        let mut take = |request: Request, newest: &mut Option<Request>| match request {
            Request::Update(update) => reader.update(update),
            other => *newest = Some(other),
        };
        if playing.is_none() {
            match requests.recv() {
                Ok(request) => take(request, &mut newest),
                Err(_) => return,
            }
        }
        loop {
            match requests.try_recv() {
                Ok(request) => take(request, &mut newest),
                Err(flume::TryRecvError::Empty) => break,
                Err(flume::TryRecvError::Disconnected) => return,
            }
        }
        if newest.is_none() && playing.is_none() {
            continue;
        }
        let rate = reader.rate();
        let last = reader.last();
        match newest {
            Some(Request::Update(_)) => continue,
            Some(Request::Show {
                time,
                size,
                generation,
                fine,
            }) => {
                playing = None;
                if let Some(mut frame) = decode(reader.as_mut(), time, size, generation, fine) {
                    frame.still = true;
                    frame.fine = fine;
                    // A newer request may be waiting; it replaces this frame.
                    if frames.send(Ok(frame)).is_err() {
                        return;
                    }
                }
                continue;
            }
            Some(Request::Play {
                from,
                step,
                size,
                generation,
            }) => {
                playing = Some(Playing {
                    next: rate.frame_at(from),
                    step,
                    size,
                    generation,
                    chunk: Vec::new(),
                });
            }
            None => {}
        }
        let Some(play) = playing.as_mut() else {
            continue;
        };
        if play.next < 0 || play.next > last {
            playing = None;
            continue;
        }
        let frame = if play.step > 0 {
            let frame = decode(
                reader.as_mut(),
                rate.time_of(play.next),
                play.size,
                play.generation,
                false,
            );
            play.next += play.step;
            frame
        } else {
            if play.chunk.is_empty() {
                match reader.frames_before(play.next, play.size) {
                    Ok(frames) => {
                        play.chunk
                            .extend(frames.into_iter().map(|(image, index)| Decoded {
                                index,
                                image: Arc::new(RenderImage::new(vec![image::Frame::new(image)])),
                                generation: play.generation,
                                still: false,
                                fine: false,
                                hardware: reader.hardware(),
                            }))
                    }
                    Err(error) => eprintln!("spectrum: {error:#}"),
                }
                // Faster than real time, only every step-th frame shows.
                let step = play.step.unsigned_abs() as i64;
                let next = play.next;
                play.chunk.retain(|frame| (next - frame.index) % step == 0);
                if play.chunk.is_empty() {
                    play.next = -1;
                    continue;
                }
            }
            let frame = play.chunk.pop();
            if let Some(frame) = &frame {
                play.next = frame.index + play.step;
            }
            frame
        };
        let Some(frame) = frame else {
            continue;
        };
        // Wait for room, but give way at once to a new request.
        let mut pending = Ok(frame);
        loop {
            match frames.send_timeout(pending, Duration::from_millis(4)) {
                Ok(()) => break,
                Err(flume::SendTimeoutError::Timeout(frame)) => {
                    if !requests.is_empty() {
                        break;
                    }
                    pending = frame;
                }
                Err(flume::SendTimeoutError::Disconnected(_)) => return,
            }
        }
    }
}

fn decode(
    reader: &mut dyn Pictures,
    time: Time,
    size: u32,
    generation: u64,
    still: bool,
) -> Option<Decoded> {
    match reader.frame(time, size, still) {
        Ok((image, index)) => Some(Decoded {
            index,
            image: Arc::new(RenderImage::new(vec![image::Frame::new(image)])),
            generation,
            still: false,
            fine: false,
            hardware: reader.hardware(),
        }),
        Err(error) => {
            eprintln!("spectrum: {error:#}");
            None
        }
    }
}
