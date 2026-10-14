//! Velocity and time remap are two ways to look at one timing: which source
//! moment shows at each moment of the clip. Converting between them keeps
//! every frame where it was, within a quarter of a frame, so a person can
//! switch views and edit in either without losing work. The new keyframes
//! fall where the old ones were, with more only where a curve needs them.
use crate::{
    Clip,
    speed::{Curve, MAX_PERCENT, RemapKey, Speed, VelocityKey, remap, velocity_area},
};
use serde::{Deserialize, Serialize};
use spectrum_time::Time;

/// Which keyframes a clip's speed is kept as.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpeedForm {
    Velocity,
    Remap,
}

/// A clip's speed as the other form, and its in point (velocity counts from
/// it). `exact` is false where velocity cannot follow a remap: a jump, or
/// faster than 1000%.
pub struct Converted {
    pub speed: Speed,
    pub source_in: Time,
    pub exact: bool,
}

/// Curves a fitted span may take, simplest first.
const CURVES: [Curve; 5] = [
    Curve::Linear,
    Curve::Smooth,
    Curve::Fast,
    Curve::Slow,
    Curve::Sharp,
];

/// How finely a span may be split before giving up on more accuracy.
const SMALLEST: f64 = 1e-3;

impl Clip {
    /// The source position at keyframe time `tau` (seconds), before it
    /// repeats: absolute for a remap, counted from the in point otherwise.
    fn position(&self, tau: f64) -> f64 {
        match &self.speed {
            Speed::Normal => self.source_in.seconds() + tau,
            Speed::Velocity { keys, origin } => {
                self.source_in.seconds() + velocity_area(keys, tau)
                    - velocity_area(keys, origin.seconds())
            }
            Speed::Remap { keys, .. } => remap(keys, tau),
        }
    }

    /// The keyframe times the conversion covers: from the clip's original
    /// start to a second past its end or last keyframe, so trims that reveal
    /// more keep their timing.
    fn span(&self) -> (f64, f64, Vec<f64>) {
        let origin = self.speed.origin().seconds();
        let mut times: Vec<f64> = match &self.speed {
            Speed::Normal => vec![],
            Speed::Velocity { keys, .. } => keys.iter().map(|k| k.at.seconds()).collect(),
            Speed::Remap { keys, .. } => keys.iter().map(|k| k.at.seconds()).collect(),
        };
        let end = times
            .iter()
            .copied()
            .fold(origin + self.duration.seconds(), f64::max)
            + 1.;
        times.extend([0., end]);
        times.sort_by(f64::total_cmp);
        times.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
        (origin, end, times)
    }

    /// The clip's speed as `form`, keeping every frame within `tolerance`
    /// source seconds.
    pub fn converted(&self, form: SpeedForm, tolerance: f64) -> Converted {
        let same = matches!(
            (&self.speed, form),
            (Speed::Velocity { .. }, SpeedForm::Velocity) | (Speed::Remap { .. }, SpeedForm::Remap)
        );
        if same {
            return Converted {
                speed: self.speed.clone(),
                source_in: self.source_in,
                exact: true,
            };
        }
        if self.speed.is_normal() && form == SpeedForm::Velocity {
            return Converted {
                speed: Speed::Velocity {
                    keys: vec![VelocityKey {
                        at: Time::ZERO,
                        percent: 100.,
                        curve: Curve::Smooth,
                    }],
                    origin: Time::ZERO,
                },
                source_in: self.source_in,
                exact: true,
            };
        }
        match form {
            SpeedForm::Remap => self.to_remap(tolerance),
            SpeedForm::Velocity => self.to_velocity(tolerance),
        }
    }

    fn to_remap(&self, tolerance: f64) -> Converted {
        let (origin, _, times) = self.span();
        let mut keys: Vec<RemapKey> = Vec::new();
        for pair in times.windows(2) {
            self.fit_remap(pair[0], pair[1], tolerance, &mut keys);
        }
        let last = *times.last().expect("times");
        keys.push(RemapKey {
            at: Time::from_seconds(last),
            source: Time::from_seconds(self.position(last)),
            curve: Curve::Linear,
        });
        let values: Vec<(Time, f64, Curve)> = keys
            .iter()
            .map(|k| (k.at, k.source.seconds(), k.curve))
            .collect();
        let keys = straightened(&values)
            .into_iter()
            .map(|(at, source, curve)| RemapKey {
                at,
                source: Time::from_seconds(source),
                curve,
            })
            .collect();
        Converted {
            speed: Speed::Remap {
                keys,
                origin: Time::from_seconds(origin),
            },
            source_in: self.source_in,
            exact: true,
        }
    }

    /// Adds keyframes for the span from `a` to `b` (not the one at `b`):
    /// the curve that follows the position best, split in half where none
    /// is close enough.
    fn fit_remap(&self, a: f64, b: f64, tolerance: f64, keys: &mut Vec<RemapKey>) {
        let (pa, pb) = (self.position(a), self.position(b));
        let error = |curve: Curve| {
            (1..32)
                .map(|i| {
                    let u = f64::from(i) / 32.;
                    let fitted = pa + (pb - pa) * curve.shape(u);
                    (fitted - self.position(a + (b - a) * u)).abs()
                })
                .fold(0., f64::max)
        };
        let (curve, worst) = CURVES
            .iter()
            .map(|&curve| (curve, error(curve)))
            .min_by(|x, y| x.1.total_cmp(&y.1))
            .expect("curves");
        if worst > tolerance && b - a > SMALLEST {
            let middle = (a + b) / 2.;
            self.fit_remap(a, middle, tolerance, keys);
            self.fit_remap(middle, b, tolerance, keys);
            return;
        }
        keys.push(RemapKey {
            at: Time::from_seconds(a),
            source: Time::from_seconds(pa),
            curve,
        });
    }

    fn to_velocity(&self, tolerance: f64) -> Converted {
        let (origin, _, times) = self.span();
        let mut fitted = Fitted {
            keys: Vec::new(),
            exact: true,
            tolerance,
        };
        for pair in times.windows(2) {
            self.fit_velocity(pair[0], pair[1], &mut fitted);
        }
        let values: Vec<(Time, f64, Curve)> = fitted
            .keys
            .iter()
            .map(|k| (k.at, k.percent, k.curve))
            .collect();
        let keys = straightened(&values)
            .into_iter()
            .map(|(at, percent, curve)| VelocityKey { at, percent, curve })
            .collect();
        let speed = Speed::Velocity {
            keys,
            origin: Time::from_seconds(origin),
        };
        Converted {
            speed,
            source_in: Time::from_seconds(self.wrap(self.position(origin))),
            exact: fitted.exact,
        }
    }

    /// Adds velocity keyframes for the span from `a` to `b`: a straight
    /// change of speed whose area lands exactly on the position at `b`, so
    /// no error builds up, split where the position strays between. Where
    /// the speed carries on from the last span, the span starts from it;
    /// otherwise the speed jumps at `a` (two keyframes at one time).
    fn fit_velocity(&self, a: f64, b: f64, fitted: &mut Fitted) {
        let span = b - a;
        let step = (span / 64.).min(1e-3);
        let (pa, pb) = (self.position(a), self.position(b));
        let start = (self.position(a + step) - pa) / step;
        let previous = fitted.keys.last().map(|k| k.percent / 100.);
        let continuing = previous.filter(|v| (v - start).abs() < 0.005);
        let va = continuing.unwrap_or(start);
        let vb = 2. * (pb - pa) / span - va;
        let limit = MAX_PERCENT / 100.;
        let error = (1..32)
            .map(|i| {
                let t = span * f64::from(i) / 32.;
                let fitted = pa + va * t + (vb - va) * t * t / (2. * span);
                (fitted - self.position(a + t)).abs()
            })
            .fold(0., f64::max);
        let fits = error <= fitted.tolerance && vb.abs() <= limit && va.abs() <= limit;
        if !fits && span > SMALLEST {
            let middle = (a + b) / 2.;
            self.fit_velocity(a, middle, fitted);
            self.fit_velocity(middle, b, fitted);
            return;
        }
        fitted.exact &= fits;
        let key = |at: f64, v: f64| VelocityKey {
            at: Time::from_seconds(at),
            percent: (v * 100.).clamp(-MAX_PERCENT, MAX_PERCENT),
            curve: Curve::Linear,
        };
        if continuing.is_none() {
            fitted.keys.push(key(a, va));
        }
        fitted.keys.push(key(b, vb));
    }
}

/// The keyframes without those that lie on a straight line between their
/// neighbors (with straight spans either side), which change nothing.
fn straightened(keys: &[(Time, f64, Curve)]) -> Vec<(Time, f64, Curve)> {
    let mut kept: Vec<(Time, f64, Curve)> = Vec::with_capacity(keys.len());
    for (index, &key) in keys.iter().enumerate() {
        let Some(&next) = keys.get(index + 1) else {
            kept.push(key);
            break;
        };
        let removable = kept.last().is_some_and(|&before| {
            let (t0, t1, t2) = (before.0.seconds(), key.0.seconds(), next.0.seconds());
            if before.2 != Curve::Linear || key.2 != Curve::Linear || t1 <= t0 || t2 <= t1 {
                return false;
            }
            let between = before.1 + (next.1 - before.1) * (t1 - t0) / (t2 - t0);
            (between - key.1).abs() <= 1e-6 * key.1.abs().max(1.)
        });
        if !removable {
            kept.push(key);
        }
    }
    kept
}

struct Fitted {
    keys: Vec<VelocityKey>,
    exact: bool,
    tolerance: f64,
}
