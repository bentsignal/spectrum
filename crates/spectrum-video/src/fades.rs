//! Fades and crossfades, as Sony Vegas makes them. A clip fades in from
//! nothing over `fade_in` and out over `fade_out`, picture and sound. A
//! crossfade is a fade in that mixes from the clip before on its track,
//! which plays on past its own end underneath (as a transition uses the
//! media past a cut); dragging a clip, or an edge, across its neighbor makes
//! one, the overlap becoming the crossfade. Tracks never hold overlapping
//! clips, so every other edit works as before.
use crate::{Clip, Track, TrackKind, Video};
use spectrum_time::Time;

/// Share of the way through a fade, eased at both ends.
fn ease(share: f64) -> f32 {
    let u = share.clamp(0., 1.);
    (u * u * (3. - 2. * u)) as f32
}

impl Clip {
    /// How much of the clip shows at `time`, from 0 to 1, by its fades.
    pub fn fade_at(&self, time: Time) -> f32 {
        let length = self.duration.0.max(1);
        let into = (time - self.start).0 as f64;
        let left = (self.end() - time).0 as f64;
        let fade_in = self.fade_in.0.min(length);
        let fade_out = self.fade_out.0.min(length);
        let rising = if fade_in > 0 {
            ease(into / fade_in as f64)
        } else {
            1.
        };
        let falling = if fade_out > 0 {
            ease(left / fade_out as f64)
        } else {
            1.
        };
        rising.min(falling)
    }

    pub fn has_fades(&self) -> bool {
        self.fade_in > Time::ZERO || self.fade_out > Time::ZERO
    }

    /// Whether `time` falls in the clip's crossfade from the clip before.
    pub fn crossfading(&self, time: Time) -> bool {
        self.crossfade && self.fade_in > Time::ZERO && time < self.start + self.fade_in
    }
}

impl Track {
    /// The clip a crossfade at `clip` mixes from: the one ending where it
    /// starts.
    pub fn before(&self, clip: &Clip) -> Option<&Clip> {
        self.clips
            .iter()
            .find(|c| c.id != clip.id && c.end() == clip.start && c.enabled)
    }
}

impl Video {
    /// The pictures at `time`, from the bottom up, each with how much of it
    /// shows: a track's clip, and under a crossfade the clip before it
    /// playing on.
    pub fn layers(&self, time: Time) -> Vec<(&Clip, f32)> {
        let mut layers = Vec::new();
        for track in self
            .tracks
            .iter()
            .filter(|t| t.kind == TrackKind::Video && !t.muted)
        {
            let Some(clip) = track.clips.iter().find(|c| c.covers(time) && c.enabled) else {
                continue;
            };
            if clip.crossfading(time)
                && let Some(before) = track.before(clip)
            {
                layers.push((before, before.opacity));
            }
            layers.push((clip, clip.opacity * clip.fade_at(time)));
        }
        layers
    }
}

/// Clips moved onto a track meet the clips there: where one overlaps the
/// end of a clip already there, that clip's tail goes and the overlap
/// becomes the moved clip's crossfade; where one overlaps the start of a
/// clip there, the moved clip's tail goes and the overlap becomes that
/// clip's crossfade. Clips it covers whole are left for overwriting.
pub(crate) fn meet(track: &mut Track, clip: &mut Clip, moving: &[u64]) {
    let mut gave = None;
    for other in track.clips.iter().filter(|c| !moving.contains(&c.id)) {
        if other.start < clip.start && other.end() > clip.start && other.end() <= clip.end() {
            let overlap = other.end() - clip.start;
            clip.fade_in = overlap;
            clip.crossfade = true;
        } else if other.start > clip.start && other.start < clip.end() && other.end() > clip.end() {
            gave = Some((other.id, clip.end() - other.start, other.start));
        }
    }
    if let Some((next, overlap, start)) = gave {
        clip.duration = start - clip.start;
        if let Some(next) = track.clips.iter_mut().find(|c| c.id == next) {
            next.fade_in = overlap;
            next.crossfade = true;
        }
    }
}
