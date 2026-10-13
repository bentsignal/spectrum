//! Sound for the player: one output stream to the default device, fed by a
//! decoding thread. The samples the device has played are the playback
//! clock, so pictures follow the sound.
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use spectrum_media::{AudioReader, CHANNELS, SAMPLE_RATE, Time};
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
    },
    time::Duration,
};

/// Where the sound thread gets sound: a file's track, or a video's mix.
pub trait Sounds: Send {
    /// Plays sound track `track` (for files with several) from `from`.
    fn start(&mut self, from: Time, track: usize) -> anyhow::Result<()>;
    /// Fills `out` with interleaved 48 kHz stereo, returning the sample
    /// frames written; fewer means the sound ended.
    fn read(&mut self, out: &mut [f32]) -> anyhow::Result<usize>;
    /// Takes a newer version of what it plays, such as an edited video.
    fn update(&mut self, _: Box<dyn std::any::Any + Send>) {}
}

/// A file's sound tracks.
struct FileSound {
    file: PathBuf,
    reader: Option<(usize, AudioReader)>,
}

impl Sounds for FileSound {
    fn start(&mut self, from: Time, track: usize) -> anyhow::Result<()> {
        if self.reader.as_ref().is_none_or(|(open, _)| *open != track) {
            self.reader = Some((track, AudioReader::open(&self.file, track)?));
        }
        if let Some((_, reader)) = &mut self.reader {
            reader.seek(from)?;
        }
        Ok(())
    }

    fn read(&mut self, out: &mut [f32]) -> anyhow::Result<usize> {
        match &mut self.reader {
            Some((_, reader)) => reader.read(out),
            None => Ok(0),
        }
    }
}

/// Decoded sound kept ahead of the device.
const AHEAD: usize = SAMPLE_RATE as usize / 2 * CHANNELS;

struct Shared {
    /// 48 kHz interleaved stereo waiting to play.
    buffer: Mutex<VecDeque<f32>>,
    /// 48 kHz sample frames played since playback started.
    played: AtomicU64,
    playing: AtomicBool,
    /// Bumped on every start and stop, so stale decoding is dropped.
    generation: AtomicU64,
    /// How long a sample takes from the callback to the speaker, in µs.
    latency: AtomicU64,
    volume: AtomicU32,
    /// No output could be opened.
    unavailable: AtomicBool,
}

enum Command {
    Start {
        from: Time,
        track: usize,
        generation: u64,
    },
    Stop,
    Update(Box<dyn std::any::Any + Send>),
}

/// A sound output and its decoder, both on their own thread: opening a
/// device can take tens of milliseconds, which the window never waits for.
pub struct AudioOut {
    shared: Arc<Shared>,
    commands: flume::Sender<Command>,
}

impl AudioOut {
    /// Starts opening the default output for `file`.
    pub fn open(file: PathBuf) -> Self {
        Self::open_with(move || Some(Box::new(FileSound { file, reader: None }) as Box<dyn Sounds>))
    }

    /// Starts opening the default output for sound made on its thread by
    /// `open`.
    pub fn open_with(open: impl FnOnce() -> Option<Box<dyn Sounds>> + Send + 'static) -> Self {
        let shared = Arc::new(Shared {
            buffer: Mutex::new(VecDeque::with_capacity(AHEAD * 2)),
            played: AtomicU64::new(0),
            playing: AtomicBool::new(false),
            generation: AtomicU64::new(0),
            latency: AtomicU64::new(0),
            volume: AtomicU32::new(1f32.to_bits()),
            unavailable: AtomicBool::new(false),
        });
        let (commands, receiver) = flume::unbounded();
        let thread = shared.clone();
        let started = std::thread::Builder::new()
            .name("spectrum-audio".into())
            .spawn(move || {
                // The stream lives on this thread, as some systems require.
                match (output_stream(&thread), open()) {
                    (Some(_stream), Some(sounds)) => decode(sounds, receiver, thread),
                    _ => thread.unavailable.store(true, Ordering::SeqCst),
                }
            });
        if started.is_err() {
            shared.unavailable.store(true, Ordering::SeqCst);
        }
        Self { shared, commands }
    }

    /// Whether this computer has no sound output to play on, as on a test
    /// machine; the picture then keeps time on its own.
    pub fn unavailable(&self) -> bool {
        self.shared.unavailable.load(Ordering::SeqCst)
    }

    /// Plays track `track` from `from`.
    pub fn start(&self, from: Time, track: usize) {
        let generation = self.shared.generation.fetch_add(1, Ordering::SeqCst) + 1;
        self.shared.playing.store(false, Ordering::SeqCst);
        lock(&self.shared.buffer).clear();
        self.shared.played.store(0, Ordering::SeqCst);
        let _ = self.commands.send(Command::Start {
            from,
            track,
            generation,
        });
    }

    pub fn stop(&self) {
        self.shared.generation.fetch_add(1, Ordering::SeqCst);
        self.shared.playing.store(false, Ordering::SeqCst);
        lock(&self.shared.buffer).clear();
        let _ = self.commands.send(Command::Stop);
    }

    /// How long the sound has played since `start`, as heard: once the
    /// first samples reach the speaker. `None` until they do.
    pub fn heard(&self) -> Option<Duration> {
        if !self.shared.playing.load(Ordering::SeqCst) {
            return None;
        }
        let played = self.shared.played.load(Ordering::SeqCst);
        let latency = Duration::from_micros(self.shared.latency.load(Ordering::Relaxed));
        Some(
            Duration::from_secs_f64(played as f64 / f64::from(SAMPLE_RATE)).saturating_sub(latency),
        )
    }

    /// Hands the sound thread a newer version of what it plays.
    pub fn update(&self, update: Box<dyn std::any::Any + Send>) {
        let _ = self.commands.send(Command::Update(update));
    }

    pub fn set_volume(&self, volume: f32) {
        self.shared
            .volume
            .store(volume.clamp(0., 1.).to_bits(), Ordering::Relaxed);
    }
}

impl Drop for AudioOut {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Opens the default device's stream, fed from `shared`.
fn output_stream(shared: &Arc<Shared>) -> Option<cpal::Stream> {
    let device = cpal::default_host().default_output_device()?;
    let config = device.default_output_config().ok()?;
    let rate = config.sample_rate();
    let channels = usize::from(config.channels()).max(1);
    let config: cpal::StreamConfig = config.into();
    let mut resampler = Resampler::new(rate);
    let output = shared.clone();
    let stream = device
        .build_output_stream(
            config,
            move |data: &mut [f32], info: &cpal::OutputCallbackInfo| {
                let stamp = info.timestamp();
                let latency = stamp.playback.duration_since(stamp.callback);
                output
                    .latency
                    .store(latency.as_micros() as u64, Ordering::Relaxed);
                resampler.fill(&output, data, channels);
            },
            |error| eprintln!("spectrum: sound output: {error}"),
            None,
        )
        .ok()?;
    stream.play().ok()?;
    Some(stream)
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

/// Keeps the buffer filled for the current start until stopped.
fn decode(mut sounds: Box<dyn Sounds>, commands: flume::Receiver<Command>, shared: Arc<Shared>) {
    let mut active: Option<u64> = None;
    let mut chunk = vec![0f32; 4096 * CHANNELS];
    loop {
        let command = if active.is_some() {
            match commands.try_recv() {
                Ok(command) => Some(command),
                Err(flume::TryRecvError::Empty) => None,
                Err(flume::TryRecvError::Disconnected) => return,
            }
        } else {
            match commands.recv() {
                Ok(command) => Some(command),
                Err(_) => return,
            }
        };
        match command {
            Some(Command::Start {
                from,
                track,
                generation,
            }) => {
                active = sounds.start(from, track).ok().map(|_| generation);
            }
            Some(Command::Stop) => active = None,
            Some(Command::Update(update)) => sounds.update(update),
            None => {}
        }
        let Some(generation) = active else {
            continue;
        };
        if lock(&shared.buffer).len() >= AHEAD {
            std::thread::sleep(Duration::from_millis(5));
            continue;
        }
        let frames = sounds.read(&mut chunk).unwrap_or(0);
        if shared.generation.load(Ordering::SeqCst) != generation {
            active = None;
            continue;
        }
        lock(&shared.buffer).extend(&chunk[..frames * CHANNELS]);
        shared.playing.store(true, Ordering::SeqCst);
        if frames * CHANNELS < chunk.len() {
            // The sound ended; the clock runs on in silence.
            active = None;
        }
    }
}

/// Plays 48 kHz stereo at the device's rate and channel count, stepping
/// through the source at fractional positions.
struct Resampler {
    step: f64,
    position: f64,
    previous: [f32; 2],
    next: [f32; 2],
}

impl Resampler {
    fn new(device_rate: u32) -> Self {
        Self {
            step: f64::from(SAMPLE_RATE) / f64::from(device_rate.max(1)),
            position: 0.,
            previous: [0.; 2],
            next: [0.; 2],
        }
    }

    fn fill(&mut self, shared: &Shared, data: &mut [f32], channels: usize) {
        let volume = f32::from_bits(shared.volume.load(Ordering::Relaxed));
        let playing = shared.playing.load(Ordering::SeqCst);
        let mut buffer = lock(&shared.buffer);
        let mut consumed = 0u64;
        for frame in data.chunks_mut(channels) {
            let sample = if playing {
                while self.position >= 1. {
                    self.position -= 1.;
                    self.previous = self.next;
                    if buffer.len() >= CHANNELS {
                        self.next = [
                            buffer.pop_front().unwrap_or(0.),
                            buffer.pop_front().unwrap_or(0.),
                        ];
                    } else {
                        self.next = [0.; 2];
                    }
                    consumed += 1;
                }
                let t = self.position as f32;
                self.position += self.step;
                [
                    self.previous[0] + (self.next[0] - self.previous[0]) * t,
                    self.previous[1] + (self.next[1] - self.previous[1]) * t,
                ]
            } else {
                self.position = 0.;
                self.previous = [0.; 2];
                self.next = [0.; 2];
                [0.; 2]
            };
            for (channel, out) in frame.iter_mut().enumerate() {
                // Mono devices get both sides; extra channels stay silent.
                *out = match (channels, channel) {
                    (1, _) => (sample[0] + sample[1]) * 0.5 * volume,
                    (_, 0 | 1) => sample[channel] * volume,
                    _ => 0.,
                };
            }
        }
        if playing {
            shared.played.fetch_add(consumed, Ordering::SeqCst);
        }
    }
}
