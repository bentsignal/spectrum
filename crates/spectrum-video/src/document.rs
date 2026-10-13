//! A video: an edit of tracks and clips. Video tracks stack (the first is
//! at the bottom); audio tracks mix. Every clip uses another asset in the
//! library and shows part of it at a place on the timeline. Times are exact
//! (`spectrum_time::Time`) and clip edges sit on the video's frames.
use serde::{Deserialize, Serialize};
use spectrum_library::AssetId;
use spectrum_time::{FrameRate, Time};

use crate::speed::Speed;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Video {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub rate: FrameRate,
    /// Video tracks first (V1 at the bottom of the picture), then audio.
    pub tracks: Vec<Track>,
    /// The next track or clip ID.
    pub next_id: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackKind {
    Video,
    Audio,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Track {
    pub id: u64,
    pub kind: TrackKind,
    pub name: String,
    /// Hidden video or muted audio does not show or play.
    #[serde(default)]
    pub muted: bool,
    /// A locked track's clips do not change.
    #[serde(default)]
    pub locked: bool,
    /// Clips in timeline order, never overlapping.
    pub clips: Vec<Clip>,
}

/// What a clip shows or plays.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Source {
    /// Footage's picture.
    Footage { asset: AssetId },
    /// A sound track of footage or audio (`track` 0 is the first).
    Sound { asset: AssetId, track: usize },
    /// An image, held for the clip's length.
    Image { asset: AssetId },
    /// A canvas, held for the clip's length.
    Canvas { asset: AssetId },
}

impl Source {
    pub fn asset(self) -> AssetId {
        match self {
            Source::Footage { asset }
            | Source::Sound { asset, .. }
            | Source::Image { asset }
            | Source::Canvas { asset } => asset,
        }
    }

    pub fn kind(self) -> TrackKind {
        match self {
            Source::Sound { .. } => TrackKind::Audio,
            _ => TrackKind::Video,
        }
    }

    /// Stills last as long as their clips; footage and sound end.
    pub fn is_still(self) -> bool {
        matches!(self, Source::Image { .. } | Source::Canvas { .. })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Clip {
    pub id: u64,
    pub name: String,
    pub source: Source,
    /// Where the clip starts on the timeline.
    pub start: Time,
    /// How long it lasts on the timeline.
    pub duration: Time,
    /// Where in the source it starts.
    pub source_in: Time,
    /// The source's length, for footage and sound.
    #[serde(default)]
    pub source_duration: Option<Time>,
    /// Clips with the same link move, trim, split, and delete together,
    /// such as footage's picture and its sound.
    #[serde(default)]
    pub link: Option<u64>,
    #[serde(default = "enabled")]
    pub enabled: bool,
    /// Picture opacity, 0 to 1.
    #[serde(default = "one")]
    pub opacity: f32,
    /// Sound level, in decibels.
    #[serde(default)]
    pub gain_db: f32,
    /// How fast the source plays.
    #[serde(default)]
    pub speed: Speed,
    /// How frames between the source's frames are made, when a clip plays
    /// slower than its source's frame rate.
    #[serde(default)]
    pub interpolation: Interpolation,
    /// The sound speeds up, slows down, and plays backward with the
    /// picture; off, it plays at normal speed from the clip's in point.
    #[serde(default = "enabled")]
    pub sound_follows_speed: bool,
}

/// Frames made between a source's frames in slow motion.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Interpolation {
    /// The nearest source frame, repeated.
    Off,
    /// The two nearest frames blended.
    Blend,
    /// New frames made by following each pixel's motion between the two
    /// nearest frames: the smoothest slow motion.
    #[default]
    Motion,
}

fn enabled() -> bool {
    true
}

fn one() -> f32 {
    1.0
}

impl Clip {
    pub fn end(&self) -> Time {
        self.start + self.duration
    }

    /// Whether the clip shows at `time`.
    pub fn covers(&self, time: Time) -> bool {
        self.start <= time && time < self.end()
    }

    /// The source time showing at timeline `time`, following the clip's
    /// speed and repeating footage that runs out.
    pub fn source_time(&self, time: Time) -> Time {
        Time::from_seconds(self.source_seconds(time))
    }

    /// The same in exact seconds, which may fall between source frames.
    pub fn source_seconds(&self, time: Time) -> f64 {
        self.wrap(self.unwrapped_seconds(time))
    }

    /// The source seconds before repeating: past the source's end when
    /// playing fast, below zero when playing backward.
    pub fn unwrapped_seconds(&self, time: Time) -> f64 {
        let offset = (time - self.start).max(Time::ZERO);
        if self.speed.is_absolute() {
            self.speed.source_seconds(offset)
        } else {
            self.source_in.seconds() + self.speed.source_seconds(offset)
        }
    }

    /// Repeats source seconds into the source's length.
    pub fn wrap(&self, seconds: f64) -> f64 {
        match self.source_duration.filter(|length| length.0 > 0) {
            Some(length) => seconds.rem_euclid(length.seconds()),
            None => seconds.max(0.),
        }
    }

    /// How much more source the clip could show past its end at normal
    /// speed, or `None` for stills (which last as long as they need).
    pub fn source_left(&self) -> Option<Time> {
        let length = self.source_duration?;
        if !self.speed.is_normal() {
            // Faster or slower clips repeat instead of running out.
            return None;
        }
        Some((length - self.source_in - self.duration).max(Time::ZERO))
    }
}

impl Video {
    pub fn new(name: impl Into<String>, width: u32, height: u32, rate: FrameRate) -> Self {
        let mut video = Self {
            name: name.into(),
            width: width.max(2),
            height: height.max(2),
            rate,
            tracks: Vec::new(),
            next_id: 1,
        };
        for kind in [
            TrackKind::Video,
            TrackKind::Video,
            TrackKind::Audio,
            TrackKind::Audio,
        ] {
            video.add_track(kind);
        }
        video
    }

    pub(crate) fn take_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// Adds a track above the others of its kind, returning its ID.
    pub fn add_track(&mut self, kind: TrackKind) -> u64 {
        let id = self.take_id();
        let number = self.tracks.iter().filter(|t| t.kind == kind).count() + 1;
        let name = match kind {
            TrackKind::Video => format!("V{number}"),
            TrackKind::Audio => format!("A{number}"),
        };
        let at = match kind {
            TrackKind::Video => self
                .tracks
                .iter()
                .rposition(|t| t.kind == TrackKind::Video)
                .map_or(0, |i| i + 1),
            TrackKind::Audio => self.tracks.len(),
        };
        self.tracks.insert(
            at,
            Track {
                id,
                kind,
                name,
                muted: false,
                locked: false,
                clips: Vec::new(),
            },
        );
        id
    }

    /// The timeline's length: where its last clip ends.
    pub fn duration(&self) -> Time {
        self.tracks
            .iter()
            .flat_map(|track| track.clips.iter().map(Clip::end))
            .max()
            .unwrap_or(Time::ZERO)
    }

    /// Rounds a time to the nearest frame of this video.
    pub fn snap(&self, time: Time) -> Time {
        let half = Time(self.rate.frame_duration().0 / 2);
        self.rate.time_of(self.rate.frame_at(time + half))
    }

    pub fn track(&self, id: u64) -> Option<&Track> {
        self.tracks.iter().find(|track| track.id == id)
    }

    pub(crate) fn track_mut(&mut self, id: u64) -> Option<&mut Track> {
        self.tracks.iter_mut().find(|track| track.id == id)
    }

    /// A clip and the track it is on.
    pub fn clip(&self, id: u64) -> Option<(&Track, &Clip)> {
        self.tracks
            .iter()
            .find_map(|track| track.clips.iter().find(|c| c.id == id).map(|c| (track, c)))
    }

    pub(crate) fn clip_mut(&mut self, id: u64) -> Option<&mut Clip> {
        self.tracks
            .iter_mut()
            .find_map(|track| track.clips.iter_mut().find(|c| c.id == id))
    }

    /// A clip with every clip linked to it.
    pub fn linked(&self, id: u64) -> Vec<u64> {
        let Some((_, clip)) = self.clip(id) else {
            return Vec::new();
        };
        match clip.link {
            None => vec![id],
            Some(link) => self
                .tracks
                .iter()
                .flat_map(|track| track.clips.iter())
                .filter(|c| c.link == Some(link))
                .map(|c| c.id)
                .collect(),
        }
    }

    /// The assets the video uses, by clip, for the library's references.
    pub fn uses(&self) -> Vec<(String, AssetId)> {
        self.tracks
            .iter()
            .flat_map(|track| track.clips.iter())
            .map(|clip| (clip.id.to_string(), clip.source.asset()))
            .collect()
    }

    /// The clips showing at `time` on enabled tracks of `kind`, bottom first.
    pub fn at(&self, time: Time, kind: TrackKind) -> Vec<&Clip> {
        self.tracks
            .iter()
            .filter(|track| track.kind == kind && !track.muted)
            .filter_map(|track| track.clips.iter().find(|clip| clip.covers(time)))
            .filter(|clip| clip.enabled)
            .collect()
    }
}
