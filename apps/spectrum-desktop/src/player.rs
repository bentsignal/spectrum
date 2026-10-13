//! Playing footage and audio. The playhead runs on the sound's clock when
//! there is sound, so pictures stay in sync; the picture thread decodes
//! ahead and each screen frame shows the newest frame due. Shortcuts follow
//! video editors: Space plays and pauses, J, K, and L shuttle, the arrows
//! step a frame, and Home and End jump to the ends.
use crate::{
    audio_out::AudioOut,
    player_decode::{self, Decoded, Request},
    workspace::{Open, Workspace},
};
use gpui::*;
use spectrum_library::AssetId;
use spectrum_media::{FrameRate, Media, Time};
use std::{
    collections::VecDeque,
    sync::Arc,
    time::{Duration, Instant},
};

/// How long to wait for the sound before the picture runs on its own.
const SOUND_GRACE: Duration = Duration::from_millis(400);
/// Frames the picture may trail the sound before decoding skips ahead.
const CATCH_UP: i64 = 4;

pub struct Player {
    pub id: AssetId,
    pub media: Media,
    /// Where the playhead is while paused, or where playback started.
    pub anchor: Time,
    /// Shuttle speed: 0 paused; 1, 2, 4, 8 forward; negative backward.
    pub speed: i32,
    started: Option<Instant>,
    /// The frame on screen and its number.
    pub shown: Option<(Arc<RenderImage>, i64)>,
    queue: VecDeque<Decoded>,
    generation: u64,
    requests: Option<flume::Sender<Request>>,
    audio: Option<AudioOut>,
    /// The sound track that plays.
    pub track: usize,
    pub muted: bool,
    /// Long edge frames decode at: the main area's, in device pixels.
    pub size: u32,
    /// When recent stills were asked for, by generation, to time scrubs.
    asked: VecDeque<(u64, Instant)>,
    /// The newest generation on screen: an older still never replaces it.
    shown_generation: u64,
    /// When the player opened, until its first frame shows.
    opened: Option<Instant>,
    /// Decodes paused frames from the original when the picture comes
    /// from a playback copy.
    sharp: Option<flume::Sender<Request>>,
    /// When to redraw the paused frame from the original, for which request.
    sharp_due: Option<(u64, Instant)>,
    /// The frame on screen is the original's.
    pub shown_sharp: bool,
    /// A playback copy finished while this player was open.
    pub copy_arrived: bool,
}

/// How long a paused frame from the copy waits before the original's
/// replaces it: long enough that a scrub never waits for the original.
const SHARP_AFTER: Duration = Duration::from_millis(120);

impl Player {
    pub fn new(
        id: AssetId,
        media: Media,
        size: u32,
        requests: Option<flume::Sender<Request>>,
        sharp: Option<flume::Sender<Request>>,
        audio: Option<AudioOut>,
    ) -> Self {
        let has_picture = requests.is_some();
        Self {
            id,
            media,
            anchor: Time::ZERO,
            speed: 0,
            started: None,
            shown: None,
            queue: VecDeque::new(),
            generation: 0,
            requests,
            audio,
            track: 0,
            muted: false,
            size,
            asked: VecDeque::new(),
            shown_generation: 0,
            opened: has_picture.then(Instant::now),
            sharp,
            sharp_due: None,
            shown_sharp: false,
            copy_arrived: false,
        }
    }

    /// Whether enough frames of playback wait to be shown.
    pub fn full(&self) -> bool {
        self.queue.len() >= player_decode::AHEAD
    }

    /// Whether the picture comes from a playback copy.
    pub fn uses_copy(&self) -> bool {
        self.sharp.is_some()
    }

    /// Shows the frame at `time`, pausing.
    pub fn seek(&mut self, time: Time) {
        self.show(time);
    }

    fn rate(&self) -> FrameRate {
        self.media
            .info
            .video
            .as_ref()
            .map_or(FrameRate::new(30, 1), |video| video.rate)
    }

    pub fn duration(&self) -> Time {
        self.media.info.duration
    }

    /// The last frame's start, where playback stops.
    pub fn end(&self) -> Time {
        match &self.media.info.video {
            Some(video) => video.rate.time_of(video.frames.max(1) as i64 - 1),
            None => self.duration(),
        }
    }

    pub fn has_picture(&self) -> bool {
        self.media.info.video.is_some()
    }

    /// Where the playhead is now.
    pub fn now(&self) -> Time {
        let Some(started) = self.started else {
            return self.anchor;
        };
        let sound = self
            .audio
            .as_ref()
            .filter(|audio| self.speed == 1 && !audio.unavailable());
        let elapsed = match sound {
            Some(audio) => match audio.heard() {
                Some(heard) => heard,
                None if started.elapsed() < SOUND_GRACE => Duration::ZERO,
                None => started.elapsed() - SOUND_GRACE,
            },
            None => started.elapsed(),
        };
        let moved = Time::from_seconds(elapsed.as_secs_f64() * f64::from(self.speed));
        (self.anchor + moved).clamp(Time::ZERO, self.end())
    }

    pub fn playing(&self) -> bool {
        self.speed != 0
    }

    pub fn timecode(&self, time: Time) -> String {
        time.timecode(self.rate())
    }

    fn request(&mut self, request: impl FnOnce(u64) -> Request) {
        self.generation += 1;
        self.queue.clear();
        if let Some(requests) = &self.requests {
            let _ = requests.send(request(self.generation));
        }
    }

    /// Shows the frame at `time`, pausing.
    fn show(&mut self, time: Time) {
        self.pause_clock();
        self.anchor = time.clamp(Time::ZERO, self.end());
        let (time, size) = (self.anchor, self.size);
        if self.asked.len() >= 64 {
            self.asked.pop_front();
        }
        self.asked.push_back((self.generation + 1, Instant::now()));
        self.request(|generation| Request::Show {
            time,
            size,
            generation,
        });
    }

    fn pause_clock(&mut self) {
        self.anchor = self.now();
        self.started = None;
        self.speed = 0;
        if let Some(audio) = &self.audio {
            audio.stop();
        }
    }

    /// Plays at shuttle `speed` from the playhead.
    fn play(&mut self, speed: i32) {
        let from = self.now();
        let at_end = from >= self.end() && speed > 0;
        let from = if at_end { Time::ZERO } else { from };
        self.pause_clock();
        if speed == 0 {
            return self.show(from);
        }
        self.anchor = from;
        self.speed = speed;
        self.started = Some(Instant::now());
        let size = self.size;
        let step = i64::from(speed);
        self.request(|generation| Request::Play {
            from,
            step,
            size,
            generation,
        });
        if speed == 1
            && let Some(audio) = &self.audio
        {
            audio.set_volume(if self.muted { 0. } else { 1. });
            audio.start(from, self.track);
        }
    }

    /// Starts decoding again from `from` at the current speed, keeping the
    /// clock and the sound running.
    fn restart_decoding(&mut self, from: Time) {
        let size = self.size;
        let step = i64::from(self.speed);
        self.request(|generation| Request::Play {
            from,
            step,
            size,
            generation,
        });
    }

    /// Takes a decoded frame from the picture thread. During a scrub, each
    /// still shows as it arrives unless a newer one already has, so the
    /// picture follows the drag.
    pub fn receive(&mut self, frame: Decoded) {
        if frame.still {
            if frame.generation <= self.shown_generation && self.shown.is_some() {
                crate::retire::drop_later(frame.image);
                return;
            }
            if let Some(&(_, asked)) = self.asked.iter().find(|(g, _)| *g == frame.generation) {
                crate::perf::record("player_frame_to_screen", asked.elapsed());
            }
            self.asked
                .retain(|(generation, _)| *generation > frame.generation);
            if let Some(opened) = self.opened.take() {
                crate::perf::record("player_open", opened.elapsed());
            }
            self.shown_generation = frame.generation;
            self.shown_sharp = !self.uses_copy();
            self.replace(frame.image, frame.index);
            if self.uses_copy() && frame.generation == self.generation {
                self.sharp_due = Some((frame.generation, Instant::now() + SHARP_AFTER));
            }
            return;
        }
        if frame.generation != self.generation {
            crate::retire::drop_later(frame.image);
            return;
        }
        self.queue.push_back(frame);
    }

    /// Takes a paused frame redrawn from the original.
    pub fn receive_sharp(&mut self, frame: Decoded) {
        if frame.generation != self.generation || self.playing() {
            crate::retire::drop_later(frame.image);
            return;
        }
        if let Some(&(_, asked)) = self.asked.iter().find(|(g, _)| *g == frame.generation) {
            crate::perf::record("player_sharp", asked.elapsed());
        }
        self.shown_sharp = true;
        self.replace(frame.image, frame.index);
    }

    /// Asks the original for the paused frame once the picture has rested.
    fn redraw_sharp(&mut self) {
        let Some((generation, due)) = self.sharp_due else {
            return;
        };
        if generation != self.generation || self.playing() {
            self.sharp_due = None;
            return;
        }
        if Instant::now() < due {
            return;
        }
        self.sharp_due = None;
        if let Some(sharp) = &self.sharp {
            let _ = sharp.send(Request::Show {
                time: self.anchor,
                size: self.size,
                generation,
            });
        }
    }

    /// Whether the frame on screen is the one last asked for and nothing
    /// is pending: a scrub has caught up.
    pub fn settled(&self) -> bool {
        !self.playing() && self.shown_generation == self.generation && self.shown.is_some()
    }

    fn replace(&mut self, image: Arc<RenderImage>, index: i64) {
        if let Some((old, _)) = self.shown.replace((image, index)) {
            crate::retire::drop_later(old);
        }
    }

    /// Shows the newest frame due while playing; stops at either end.
    /// Returns whether frames should keep coming.
    pub fn tick(&mut self) -> bool {
        if !self.playing() {
            return false;
        }
        let now = self.now();
        let due = self.rate().frame_at(now);
        let forward = self.speed > 0;
        let mut newest = None;
        while let Some(front) = self.queue.front() {
            let ready = if forward {
                front.index <= due
            } else {
                front.index >= due
            };
            if !ready {
                break;
            }
            if let Some(skipped) = newest.replace(self.queue.pop_front().expect("front")) {
                crate::retire::drop_later(skipped.image);
            }
        }
        let shown = self.shown.as_ref().map(|(_, index)| *index);
        if self.speed.abs() == 1 {
            // Frames the screen never drew: its refresh rate, or main
            // thread stalls.
            if let (Some(frame), Some(shown)) = (&newest, shown) {
                let skipped = ((frame.index - shown).abs() - 1).max(0);
                crate::perf::record_value("player_skipped_frames", skipped as f64);
            }
            // A frame due that the picture thread had not decoded yet.
            let behind = match &newest {
                Some(frame) => (due - frame.index).abs(),
                None => shown.map_or(0, |shown| (due - shown).abs()),
            };
            crate::perf::record_value("player_frames_behind", behind as f64);
            // Too slow to show every frame: skip ahead and stay in sync,
            // once the last skip has shown a frame.
            if behind > CATCH_UP
                && self.queue.is_empty()
                && self.shown_generation == self.generation
            {
                crate::perf::record_value("player_catch_up", 1.);
                let lead = self
                    .rate()
                    .time_of(due + i64::from(self.speed.signum()) * 2);
                self.restart_decoding(lead);
            }
        }
        if let Some(frame) = newest {
            self.shown_generation = frame.generation;
            self.shown_sharp = !self.uses_copy();
            self.replace(frame.image, frame.index);
        }
        let finished = (forward && now >= self.end()) || (!forward && now <= Time::ZERO);
        if finished && self.queue.is_empty() {
            self.pause_clock();
            return false;
        }
        true
    }
}

impl Workspace {
    /// The long edge the player decodes at: the main area's, in device pixels.
    pub(crate) fn player_size(&self, window: &Window) -> u32 {
        let viewport = window.viewport_size();
        let scale = window.scale_factor();
        let width = f32::from(viewport.width) - crate::workspace::SIDEBAR_WIDTH;
        let height = f32::from(viewport.height) - crate::workspace::HEADER_HEIGHT;
        ((width.max(height) * scale) as u32).clamp(320, 8192)
    }

    /// Runs the player each frame: shows the frame due while playing, and
    /// re-decodes at a new size when the window changes.
    pub fn step_player(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let size = self.player_size(window);
        if self
            .player
            .as_ref()
            .is_some_and(|p| p.copy_arrived && !p.playing())
        {
            self.switch_to_copy(window, cx);
        }
        let Some(player) = self.player.as_mut() else {
            return;
        };
        if let Ok(store) = &self.store {
            // Copies wait while the player plays.
            store
                .copies
                .hold
                .store(player.playing(), std::sync::atomic::Ordering::Relaxed);
        }
        if player.size != size && player.has_picture() {
            player.size = size;
            if player.playing() {
                let speed = player.speed;
                player.play(speed);
            } else {
                player.show(player.anchor);
            }
        }
        player.redraw_sharp();
        if player.tick() || player.sharp_due.is_some() {
            window.request_animation_frame();
        }
    }

    fn with_player(&mut self, cx: &mut Context<Self>, act: impl FnOnce(&mut Player)) -> bool {
        if !matches!(self.open, Open::Media(_)) {
            return false;
        }
        let Some(player) = self.player.as_mut() else {
            return false;
        };
        act(player);
        cx.notify();
        true
    }

    pub fn toggle_play(&mut self, cx: &mut Context<Self>) -> bool {
        self.with_player(cx, |player| {
            let speed = if player.playing() { 0 } else { 1 };
            player.play(speed);
        })
    }

    /// J and L: each press doubles the speed in that direction, up to 8x;
    /// K pauses.
    pub fn shuttle(&mut self, direction: i32, cx: &mut Context<Self>) -> bool {
        self.with_player(cx, |player| {
            let speed = match direction {
                0 => 0,
                _ if player.speed.signum() == direction => (player.speed * 2).clamp(-8, 8),
                _ => direction,
            };
            player.play(speed);
        })
    }

    /// Steps `frames` frames from the playhead, pausing.
    pub fn step_frames(&mut self, frames: i64, cx: &mut Context<Self>) -> bool {
        self.with_player(cx, |player| {
            let rate = player.rate();
            let target = rate.time_of(rate.frame_at(player.now()) + frames);
            player.show(target);
        })
    }

    /// Moves the playhead to `time`, pausing: from the scrub bar, Home and
    /// End, or a typed time.
    pub fn seek_player(&mut self, time: Time, cx: &mut Context<Self>) -> bool {
        self.with_player(cx, |player| player.show(time))
    }

    /// Picks the sound track that plays.
    pub fn choose_track(&mut self, track: usize, cx: &mut Context<Self>) {
        self.with_player(cx, |player| {
            player.track = track;
            if player.speed == 1 {
                player.play(1);
            }
        });
    }

    /// The player's one-key shortcuts while no field has focus. Returns
    /// whether the key was the player's.
    pub fn player_key(&mut self, key: &Keystroke, cx: &mut Context<Self>) -> bool {
        let plain = !key.modifiers.platform && !key.modifiers.control && !key.modifiers.alt;
        if !plain || !matches!(self.open, Open::Media(_)) {
            return false;
        }
        match key.key.as_str() {
            "space" => self.toggle_play(cx),
            "j" => self.shuttle(-1, cx),
            "k" => self.shuttle(0, cx),
            "l" => self.shuttle(1, cx),
            "home" => self.seek_player(Time::ZERO, cx),
            "end" => {
                let end = self.player.as_ref().map_or(Time::ZERO, |p| p.end());
                self.seek_player(end, cx)
            }
            _ => false,
        }
    }

    pub fn toggle_mute(&mut self, cx: &mut Context<Self>) {
        self.with_player(cx, |player| {
            player.muted = !player.muted;
            if let Some(audio) = &player.audio {
                audio.set_volume(if player.muted { 0. } else { 1. });
            }
        });
    }
}
