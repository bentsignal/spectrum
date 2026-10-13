//! The player's picture thread. It owns the file's decoder: shown a time, it
//! decodes that frame; playing, it decodes ahead in the direction of play
//! and hands frames over as they are ready. Only the newest request
//! counts, so a fast scrub never queues stale work.
use gpui::RenderImage;
use spectrum_media::{FrameReader, Time};
use std::{path::PathBuf, sync::Arc, time::Duration};

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
    },
    /// Decode ahead from `from`, `step` frames at a time (negative plays
    /// backward).
    Play {
        from: Time,
        step: i64,
        size: u32,
        generation: u64,
    },
}

pub struct Decoded {
    pub image: Arc<RenderImage>,
    pub index: i64,
    pub generation: u64,
    /// Answers a `Show`, rather than a frame of playback.
    pub still: bool,
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
    let (requests, incoming) = flume::unbounded();
    let (frames, received) = flume::bounded(AHEAD);
    std::thread::Builder::new()
        .name("spectrum-frames".into())
        .spawn(move || match FrameReader::open(&file) {
            Ok(reader) => {
                let reader = reader.bgra();
                let reader = match rotation {
                    Some(rotation) => reader.with_rotation(rotation),
                    None => reader,
                };
                run(reader, incoming, frames)
            }
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
    mut reader: FrameReader,
    requests: flume::Receiver<Request>,
    frames: flume::Sender<Delivery>,
) {
    let rate = reader.rate();
    let last = reader.frames().max(1) as i64 - 1;
    let mut playing: Option<Playing> = None;
    loop {
        // Take the newest request; block only while idle.
        let mut newest = None;
        if playing.is_none() {
            match requests.recv() {
                Ok(request) => newest = Some(request),
                Err(_) => return,
            }
        }
        loop {
            match requests.try_recv() {
                Ok(request) => newest = Some(request),
                Err(flume::TryRecvError::Empty) => break,
                Err(flume::TryRecvError::Disconnected) => return,
            }
        }
        match newest {
            Some(Request::Show {
                time,
                size,
                generation,
            }) => {
                playing = None;
                if let Some(mut frame) = decode(&mut reader, time, size, generation) {
                    frame.still = true;
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
                &mut reader,
                rate.time_of(play.next),
                play.size,
                play.generation,
            );
            play.next += play.step;
            frame
        } else {
            if play.chunk.is_empty() {
                match reader.frames_before(play.next, REVERSE_CHUNK, Some(play.size)) {
                    Ok(frames) => play.chunk.extend(frames.into_iter().map(|frame| Decoded {
                        index: frame.index,
                        image: Arc::new(RenderImage::new(vec![image::Frame::new(frame.image)])),
                        generation: play.generation,
                        still: false,
                    })),
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

fn decode(reader: &mut FrameReader, time: Time, size: u32, generation: u64) -> Option<Decoded> {
    match reader.frame_at(time, Some(size)) {
        Ok(frame) => Some(Decoded {
            index: frame.index,
            image: Arc::new(RenderImage::new(vec![image::Frame::new(frame.image)])),
            generation,
            still: false,
        }),
        Err(error) => {
            eprintln!("spectrum: {error:#}");
            None
        }
    }
}
