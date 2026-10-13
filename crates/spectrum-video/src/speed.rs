//! How fast a clip plays its source. A clip uses velocity keyframes (Sony
//! Vegas: percentages over the clip's time, -1000% to 1000%, so it can play
//! backward or freeze) or a time remap (After Effects: keyframes that say
//! which source time shows when), never both. Neither changes the clip's
//! length on the timeline; footage that runs out repeats.
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use spectrum_time::Time;

/// How a value moves from one keyframe to the next, as Vegas names them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Curve {
    /// A straight line.
    Linear,
    /// Leaves quickly, arrives gently.
    Fast,
    /// Leaves gently, arrives quickly.
    Slow,
    /// Gentle at both ends.
    #[default]
    Smooth,
    /// Holds near each end and turns quickly in the middle.
    Sharp,
    /// Holds until the next keyframe.
    Hold,
}

impl Curve {
    /// The share of the change made at `u` of the way between keyframes.
    pub fn shape(self, u: f64) -> f64 {
        let u = u.clamp(0., 1.);
        match self {
            Curve::Linear => u,
            Curve::Fast => 1. - (1. - u).powi(2),
            Curve::Slow => u * u,
            Curve::Smooth => u * u * (3. - 2. * u),
            Curve::Sharp => {
                if u < 0.5 {
                    4. * u.powi(3)
                } else {
                    1. - 4. * (1. - u).powi(3)
                }
            }
            Curve::Hold => 0.,
        }
    }

    /// The curve that makes the same change backward: Fast and Slow swap.
    /// Hold has no mirror; reversing holds at the other end instead.
    fn mirrored(self) -> Self {
        match self {
            Curve::Fast => Curve::Slow,
            Curve::Slow => Curve::Fast,
            other => other,
        }
    }

    /// The area under `shape` from 0 to `u`, exactly.
    fn area(self, u: f64) -> f64 {
        let u = u.clamp(0., 1.);
        match self {
            Curve::Linear => u * u / 2.,
            Curve::Fast => u - (1. - (1. - u).powi(3)) / 3.,
            Curve::Slow => u.powi(3) / 3.,
            Curve::Smooth => u.powi(3) - u.powi(4) / 2.,
            Curve::Sharp => {
                if u < 0.5 {
                    u.powi(4)
                } else {
                    // The first half's area, then the rest of the curve.
                    let half = 0.5f64.powi(4);
                    let rest = |x: f64| x + (1. - x).powi(4);
                    half + rest(u) - rest(0.5)
                }
            }
            Curve::Hold => 0.,
        }
    }
}

/// A velocity keyframe: from `at` (into the clip), the clip plays at
/// `percent` of normal speed, moving to the next keyframe along `curve`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct VelocityKey {
    pub at: Time,
    pub percent: f64,
    #[serde(default)]
    pub curve: Curve,
}

/// A time remap keyframe: at `at` (into the clip), the clip shows `source`
/// (from its in point), moving to the next keyframe along `curve`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct RemapKey {
    pub at: Time,
    pub source: Time,
    #[serde(default)]
    pub curve: Curve,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Speed {
    #[default]
    Normal,
    /// Keyframe times count from the clip's original start; `origin` is
    /// how far the clip's start now is past it (after trims and cuts), so
    /// trimming never bends a curve.
    Velocity {
        keys: Vec<VelocityKey>,
        #[serde(default)]
        origin: Time,
    },
    /// Keyframe sources are times in the source file, as After Effects
    /// has them.
    Remap {
        keys: Vec<RemapKey>,
        #[serde(default)]
        origin: Time,
    },
}

/// Velocity's limits, as Vegas has them.
pub const MAX_PERCENT: f64 = 1000.;

impl Speed {
    /// Checks keyframes from a command: in time order, within limits, at
    /// least one.
    pub fn checked(self) -> Result<Self> {
        match &self {
            Speed::Normal => {}
            Speed::Velocity { keys, .. } => {
                if keys.is_empty() {
                    bail!("velocity needs at least one keyframe");
                }
                for key in keys {
                    if !key.percent.is_finite() || key.percent.abs() > MAX_PERCENT {
                        bail!("velocity runs from -1000% to 1000%");
                    }
                    if key.at < Time::ZERO {
                        bail!("keyframes start at the clip's start");
                    }
                }
                if keys.windows(2).any(|pair| pair[0].at > pair[1].at) {
                    bail!("velocity keyframes must be in time order");
                }
            }
            Speed::Remap { keys, .. } => {
                if keys.is_empty() {
                    bail!("a time remap needs at least one keyframe");
                }
                if keys.iter().any(|key| key.at < Time::ZERO) {
                    bail!("keyframes start at the clip's start");
                }
                if keys.windows(2).any(|pair| pair[0].at > pair[1].at) {
                    bail!("time remap keyframes must be in time order");
                }
            }
        }
        Ok(self)
    }

    pub fn is_normal(&self) -> bool {
        matches!(self, Speed::Normal)
    }

    /// How far into the source, from the clip's in point, the clip is at
    /// `offset` from its start, in exact seconds (it may fall between
    /// frames, or be negative when playing backward). For a time remap,
    /// the source time itself.
    pub fn source_seconds(&self, offset: Time) -> f64 {
        let t = offset.seconds();
        match self {
            Speed::Normal => t,
            Speed::Velocity { keys, origin } => {
                let o = origin.seconds();
                velocity_area(keys, o + t) - velocity_area(keys, o)
            }
            Speed::Remap { keys, origin } => remap(keys, origin.seconds() + t),
        }
    }

    /// Whether the source time is set outright (a time remap) rather than
    /// counted from the clip's in point.
    pub fn is_absolute(&self) -> bool {
        matches!(self, Speed::Remap { .. })
    }

    /// The same, as a time.
    pub fn source_offset(&self, offset: Time, _duration: Time) -> Time {
        match self {
            Speed::Normal => offset,
            _ => Time::from_seconds(self.source_seconds(offset)),
        }
    }

    /// How fast the source moves at `offset`: 1 at normal speed, negative
    /// backward, 0 frozen.
    pub fn rate(&self, offset: Time) -> f64 {
        let t = offset.seconds();
        match self {
            Speed::Normal => 1.,
            Speed::Velocity { keys, origin } => velocity(keys, origin.seconds() + t) / 100.,
            Speed::Remap { keys, origin } => {
                let t = origin.seconds() + t;
                let (a, b) = ((t - 1e-3).max(0.), t + 1e-3);
                (remap(keys, b) - remap(keys, a)) / (b - a)
            }
        }
    }

    /// The clip's start moved `by` later: the keyframes stay where they
    /// were on the timeline. Returns how much further into the source the
    /// clip now starts (for counted speeds).
    pub(crate) fn shift(&mut self, by: Time) -> Time {
        let moved = match self {
            Speed::Remap { .. } => Time::ZERO,
            _ => Time::from_seconds(self.source_seconds(by)),
        };
        match self {
            Speed::Normal => return by,
            Speed::Velocity { origin, .. } | Speed::Remap { origin, .. } => *origin = *origin + by,
        }
        moved
    }

    /// Where the clip's start is in its keyframes' time.
    pub fn origin(&self) -> Time {
        match self {
            Speed::Normal => Time::ZERO,
            Speed::Velocity { origin, .. } | Speed::Remap { origin, .. } => *origin,
        }
    }
}

/// A keyframe of either kind: its time, value (a percent, or source
/// seconds), and curve.
type Key = (Time, f64, Curve);

impl Speed {
    fn keys(&self) -> Vec<Key> {
        match self {
            Speed::Normal => vec![(Time::ZERO, 100., Curve::Linear)],
            Speed::Velocity { keys, .. } => {
                keys.iter().map(|k| (k.at, k.percent, k.curve)).collect()
            }
            Speed::Remap { keys, .. } => keys
                .iter()
                .map(|k| (k.at, k.source.seconds(), k.curve))
                .collect(),
        }
    }

    /// The same kind of speed with `keys`; normal speed becomes velocity.
    fn with(&self, keys: Vec<Key>, origin: Time) -> Speed {
        match self {
            Speed::Remap { .. } => Speed::Remap {
                keys: keys
                    .into_iter()
                    .map(|(at, source, curve)| RemapKey {
                        at,
                        source: Time::from_seconds(source),
                        curve,
                    })
                    .collect(),
                origin,
            },
            _ => Speed::Velocity {
                keys: keys
                    .into_iter()
                    .map(|(at, percent, curve)| VelocityKey { at, percent, curve })
                    .collect(),
                origin,
            },
        }
    }

    /// The value at keyframe time `t`: a percent, or source seconds.
    fn value(&self, t: f64) -> f64 {
        match self {
            Speed::Normal => 100.,
            Speed::Velocity { keys, .. } => velocity(keys, t),
            Speed::Remap { keys, .. } => remap(keys, t),
        }
    }

    /// The frame `offset` into the clip held for `length` (or to the
    /// clip's end), after which the clip goes on as it was; the clip's
    /// length stays the same. Normal speed becomes velocity.
    pub fn frozen(&self, offset: Time, length: Option<Time>) -> Speed {
        let origin = self.origin();
        let at = origin + offset.max(Time::ZERO);
        let value = self.value(at.seconds());
        let keys = self.keys();
        // The curve the frozen moment was on carries on after the hold.
        let curve = keys
            .iter()
            .rev()
            .find(|k| k.0 <= at)
            .map_or(Curve::Linear, |k| k.2);
        let mut frozen: Vec<Key> = keys.iter().copied().filter(|k| k.0 < at).collect();
        if self.is_absolute() {
            frozen.push((at, value, Curve::Hold));
        } else {
            // Straight from the speed it had to a standstill.
            frozen.push((at, value, Curve::Linear));
            frozen.push((at, 0., Curve::Hold));
        }
        if let Some(length) = length {
            frozen.push((at + length, value, curve));
            frozen.extend(
                keys.iter()
                    .filter(|k| k.0 > at)
                    .map(|&(t, v, c)| (t + length, v, c)),
            );
        }
        self.with(frozen, origin)
    }

    /// The clip's frames played backward: the speed mirrored about the
    /// clip's last frame (`duration` less one `frame`), so the clip starts
    /// on the frame it ended on. Returns the speed and how many seconds
    /// further into the source the clip's in point moves (for velocity).
    pub fn reversed(&self, duration: Time, frame: Time) -> (Speed, f64) {
        let origin = self.origin();
        let pivot = (duration - frame).max(Time::ZERO);
        let keys = self.keys();
        // Keyframe times turn around `origin + pivot`, moved later if any
        // would fall before zero.
        let latest = keys.iter().map(|k| k.0).max().unwrap_or(Time::ZERO);
        let shift = (latest - origin - pivot).max(Time::ZERO);
        let turn = |t: Time| origin + pivot + shift - t;
        // Velocity turns around; source times stay. (Adding zero keeps a
        // standstill at 0 rather than -0.)
        let sign = if self.is_absolute() { 1. } else { -1. };
        let sign = move |value: f64| sign * value + 0.;
        let mut mirrored: Vec<Key> = Vec::new();
        for (index, &(at, value, _)) in keys.iter().enumerate().rev() {
            // The span before this keyframe, played backward from it.
            let before = index.checked_sub(1).map(|i| keys[i]);
            match before {
                Some((held_at, held, Curve::Hold)) => {
                    // A hold covered up to this keyframe, which showed its
                    // own value; backward, it shows that value, then holds
                    // a flick later.
                    let after = (turn(at) + Time(1)).min(turn(held_at));
                    mirrored.push((turn(at), sign(value), Curve::Hold));
                    mirrored.push((after, sign(held), Curve::Hold));
                }
                Some((.., curve)) => mirrored.push((turn(at), sign(value), curve.mirrored())),
                None => mirrored.push((turn(at), sign(value), Curve::Smooth)),
            }
        }
        // One velocity holds everywhere; keep its keyframe at the start.
        if let [only] = mirrored.as_mut_slice() {
            only.0 = shift;
        }
        let moved = match self {
            Speed::Remap { .. } => 0.,
            _ => self.source_seconds(pivot),
        };
        (self.with(mirrored, shift), moved)
    }
}

/// The velocity percent at `t` seconds into the clip.
fn velocity(keys: &[VelocityKey], t: f64) -> f64 {
    let Some(first) = keys.first() else {
        return 100.;
    };
    if t <= first.at.seconds() {
        return first.percent;
    }
    for pair in keys.windows(2) {
        let (a, b) = (&pair[0], &pair[1]);
        let (t0, t1) = (a.at.seconds(), b.at.seconds());
        if t < t1 {
            let u = if t1 > t0 { (t - t0) / (t1 - t0) } else { 1. };
            return a.percent + (b.percent - a.percent) * a.curve.shape(u);
        }
    }
    keys.last().map_or(100., |k| k.percent)
}

/// Seconds of source played from the clip's start to `t`: the area under
/// the velocity curve, worked out exactly segment by segment.
fn velocity_area(keys: &[VelocityKey], t: f64) -> f64 {
    let Some(first) = keys.first() else {
        return t;
    };
    let mut area = 0.;
    // Before the first keyframe the clip plays at its velocity.
    let start = first.at.seconds().min(t);
    area += start * first.percent / 100.;
    for pair in keys.windows(2) {
        let (a, b) = (&pair[0], &pair[1]);
        let (t0, t1) = (a.at.seconds(), b.at.seconds());
        if t <= t0 {
            return area;
        }
        let end = t.min(t1);
        let span = t1 - t0;
        if span > 0. {
            let u = (end - t0) / span;
            area +=
                ((end - t0) * a.percent + (b.percent - a.percent) * span * a.curve.area(u)) / 100.;
        }
        if t <= t1 {
            return area;
        }
    }
    // After the last keyframe the clip keeps its velocity.
    let last = keys.last().expect("keys");
    area + (t - last.at.seconds()).max(0.) * last.percent / 100.
}

/// The source seconds a time remap shows at `t` seconds into the clip.
fn remap(keys: &[RemapKey], t: f64) -> f64 {
    let Some(first) = keys.first() else {
        return t;
    };
    if t <= first.at.seconds() {
        return first.source.seconds();
    }
    for pair in keys.windows(2) {
        let (a, b) = (&pair[0], &pair[1]);
        let (t0, t1) = (a.at.seconds(), b.at.seconds());
        if t < t1 {
            let u = if t1 > t0 { (t - t0) / (t1 - t0) } else { 1. };
            return a.source.seconds()
                + (b.source.seconds() - a.source.seconds()) * a.curve.shape(u);
        }
    }
    keys.last().map_or(t, |k| k.source.seconds())
}
