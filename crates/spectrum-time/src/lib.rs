//! Time on Spectrum's timelines. A [`Time`] counts flicks, 1/705,600,000 of
//! a second, which divide evenly into every common frame rate (23.976 to
//! 120, NTSC rates included) and audio sample rate (8 to 192 kHz), so
//! frames and samples land on exact times and sums never drift.
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::ops::{Add, Neg, Sub};

/// Flicks in one second.
pub const FLICKS_PER_SECOND: i64 = 705_600_000;

/// A point or span of time, in flicks.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct Time(pub i64);

impl Time {
    pub const ZERO: Self = Self(0);

    pub fn from_seconds(seconds: f64) -> Self {
        Self((seconds * FLICKS_PER_SECOND as f64).round() as i64)
    }

    /// `numerator / denominator` seconds, rounded to the nearest flick.
    pub fn from_ratio(numerator: i64, denominator: i64) -> Self {
        if denominator == 0 {
            return Self::ZERO;
        }
        let flicks = i128::from(numerator) * i128::from(FLICKS_PER_SECOND);
        Self(div_round(flicks, i128::from(denominator)) as i64)
    }

    /// The time of sample `index` at `rate` samples a second.
    pub fn from_samples(index: i64, rate: u32) -> Self {
        Self::from_ratio(index, i64::from(rate))
    }

    pub fn seconds(self) -> f64 {
        self.0 as f64 / FLICKS_PER_SECOND as f64
    }

    /// The sample at or before this time at `rate` samples a second.
    pub fn samples(self, rate: u32) -> i64 {
        (i128::from(self.0) * i128::from(rate)).div_euclid(i128::from(FLICKS_PER_SECOND)) as i64
    }

    /// Parses a typed time: seconds (`1.5`, `1.5s`), frames (`36f`),
    /// minutes and seconds (`1:02.5`), or timecode (`00:01:02:12`). Frames
    /// and timecode need the frame rate.
    pub fn parse(text: &str, rate: Option<FrameRate>) -> Result<Self> {
        let text = text.trim();
        let need_rate = || rate.context("frames need a frame rate");
        if let Some(frames) = text.strip_suffix('f') {
            let frames: i64 = frames
                .trim()
                .parse()
                .context("frames must be a whole number")?;
            return Ok(need_rate()?.time_of(frames));
        }
        let text = text.strip_suffix('s').unwrap_or(text).trim();
        let parts: Vec<&str> = text.split(':').collect();
        let number = |part: &str| -> Result<f64> {
            let value: f64 = part
                .parse()
                .with_context(|| format!("{part} is not a number"))?;
            if !value.is_finite() || value < 0.0 {
                bail!("{part} is not a time");
            }
            Ok(value)
        };
        let negative = text.starts_with('-');
        let time = match parts.as_slice() {
            [seconds] => return Ok(Self::from_seconds(seconds.parse().context("not a time")?)),
            [minutes, seconds] => Self::from_seconds(number(minutes)? * 60.0 + number(seconds)?),
            [hours, minutes, seconds] => Self::from_seconds(
                number(hours)? * 3600.0 + number(minutes)? * 60.0 + number(seconds)?,
            ),
            [hours, minutes, seconds, frames] => {
                let rate = need_rate()?;
                let whole =
                    (number(hours)? * 3600.0 + number(minutes)? * 60.0 + number(seconds)?) as i64;
                let frames = number(frames)? as i64;
                if frames >= i64::from(rate.nominal()) {
                    bail!("timecode frame {frames} is past the last frame of a second");
                }
                rate.time_of(whole * i64::from(rate.nominal()) + frames)
            }
            _ => bail!("{text} is not a time"),
        };
        if negative {
            bail!("times cannot be negative");
        }
        Ok(time)
    }

    /// `HH:MM:SS:FF` at `rate`, counting `rate.nominal()` frames a second.
    pub fn timecode(self, rate: FrameRate) -> String {
        let sign = if self.0 < 0 { "-" } else { "" };
        let frame = rate.frame_at(Time(self.0.abs()));
        let nominal = i64::from(rate.nominal());
        let (seconds, frames) = (frame / nominal, frame % nominal);
        format!(
            "{sign}{:02}:{:02}:{:02}:{frames:02}",
            seconds / 3600,
            seconds / 60 % 60,
            seconds % 60
        )
    }
}

impl Add for Time {
    type Output = Self;
    fn add(self, other: Self) -> Self {
        Self(self.0.saturating_add(other.0))
    }
}

impl Sub for Time {
    type Output = Self;
    fn sub(self, other: Self) -> Self {
        Self(self.0.saturating_sub(other.0))
    }
}

impl Neg for Time {
    type Output = Self;
    fn neg(self) -> Self {
        Self(-self.0)
    }
}

/// Frames a second as an exact fraction, such as 30000/1001 for 29.97.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FrameRate {
    pub numerator: u32,
    pub denominator: u32,
}

impl FrameRate {
    pub const fn new(numerator: u32, denominator: u32) -> Self {
        Self {
            numerator,
            denominator,
        }
    }

    pub fn fps(self) -> f64 {
        f64::from(self.numerator) / f64::from(self.denominator.max(1))
    }

    /// Whole frames counted per second in timecode: 30 for 29.97.
    pub fn nominal(self) -> u32 {
        (self.fps().round() as u32).max(1)
    }

    /// When frame `index` starts.
    pub fn time_of(self, index: i64) -> Time {
        Time::from_ratio(
            index.saturating_mul(i64::from(self.denominator)),
            i64::from(self.numerator.max(1)),
        )
    }

    /// The frame showing at `time`: the last one starting at or before it.
    pub fn frame_at(self, time: Time) -> i64 {
        let numerator = i128::from(self.numerator.max(1));
        let per = i128::from(FLICKS_PER_SECOND) * i128::from(self.denominator.max(1));
        // A frame's start rounded to within half a flick still shows that
        // frame.
        (2 * i128::from(time.0) * numerator + numerator).div_euclid(2 * per) as i64
    }

    pub fn frame_duration(self) -> Time {
        self.time_of(1)
    }

    /// A typed rate: frames a second ("29.97", snapped to the exact rate)
    /// or a fraction ("30000/1001").
    pub fn from_text(text: &str) -> Result<Self> {
        let rate = match text.trim().trim_end_matches("fps").trim().split_once('/') {
            Some((n, d)) => FrameRate::new(n.trim().parse()?, d.trim().parse()?),
            None => {
                let fps: f64 = text
                    .trim()
                    .trim_end_matches("fps")
                    .trim()
                    .parse()
                    .context("not a frame rate")?;
                FrameRate::new((fps * 1000.).round() as u32, 1000).standardized()
            }
        };
        if rate.numerator == 0 || rate.denominator == 0 {
            bail!("{text} is not a frame rate");
        }
        Ok(rate)
    }

    /// A rate within 0.1% of a common one snaps to it, so a file that
    /// reports 29.97003 runs at exactly 30000/1001.
    pub fn standardized(self) -> Self {
        const COMMON: [(u32, u32); 12] = [
            (24000, 1001),
            (24, 1),
            (25, 1),
            (30000, 1001),
            (30, 1),
            (48, 1),
            (50, 1),
            (60000, 1001),
            (60, 1),
            (100, 1),
            (120000, 1001),
            (120, 1),
        ];
        // The nearest one: 24 and 23.976 are themselves 0.1% apart.
        let fps = self.fps();
        let gap = |rate: &Self| (rate.fps() - fps).abs() / rate.fps();
        COMMON
            .iter()
            .map(|&(n, d)| Self::new(n, d))
            .filter(|rate| gap(rate) < 0.001)
            .min_by(|a, b| gap(a).total_cmp(&gap(b)))
            .unwrap_or(self)
    }
}

impl std::fmt::Display for FrameRate {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let fps = self.fps();
        if (fps - fps.round()).abs() < 1e-9 {
            write!(formatter, "{fps:.0} fps")
        } else {
            write!(formatter, "{fps:.3} fps")
        }
    }
}

fn div_round(value: i128, divisor: i128) -> i128 {
    let (value, divisor) = if divisor < 0 {
        (-value, -divisor)
    } else {
        (value, divisor)
    };
    (value * 2 + divisor).div_euclid(divisor * 2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn common_rates_land_on_exact_flicks() {
        for (n, d) in [
            (24000, 1001),
            (25, 1),
            (30000, 1001),
            (60000, 1001),
            (120, 1),
        ] {
            let rate = FrameRate::new(n, d);
            let one = rate.frame_duration();
            assert_eq!(
                i128::from(one.0) * i128::from(n),
                i128::from(FLICKS_PER_SECOND) * i128::from(d)
            );
            for frame in [0, 1, 59, 3600, 1_000_003] {
                assert_eq!(rate.frame_at(rate.time_of(frame)), frame);
                assert_eq!(rate.frame_at(rate.time_of(frame + 1) - Time(1)), frame);
            }
        }
        for rate in [44_100, 48_000, 96_000] {
            assert_eq!(
                Time::from_samples(rate as i64, rate),
                Time(FLICKS_PER_SECOND)
            );
            assert_eq!(Time::from_samples(12_345, rate).samples(rate), 12_345);
        }
    }

    #[test]
    fn typed_times_and_timecode() {
        let rate = FrameRate::new(30000, 1001);
        assert_eq!(Time::parse("1.5", None).unwrap(), Time::from_seconds(1.5));
        assert_eq!(Time::parse("2s", None).unwrap(), Time::from_seconds(2.0));
        assert_eq!(
            Time::parse("1:02.5", None).unwrap(),
            Time::from_seconds(62.5)
        );
        assert_eq!(Time::parse("36f", Some(rate)).unwrap(), rate.time_of(36));
        let time = Time::parse("00:01:02:12", Some(rate)).unwrap();
        assert_eq!(time, rate.time_of(62 * 30 + 12));
        assert_eq!(time.timecode(rate), "00:01:02:12");
        assert!(Time::parse("00:00:01:30", Some(rate)).is_err());
        assert!(Time::parse("12f", None).is_err());
        assert!(Time::parse("-1:00", None).is_err());
        assert_eq!(
            FrameRate::new(2997003, 100000).standardized(),
            FrameRate::new(30000, 1001)
        );
        // Typed rates land on the nearest standard one.
        for (typed, rate) in [
            ("24", FrameRate::new(24, 1)),
            ("23.976", FrameRate::new(24000, 1001)),
            ("29.97", FrameRate::new(30000, 1001)),
            ("59.94 fps", FrameRate::new(60000, 1001)),
            ("30000/1001", FrameRate::new(30000, 1001)),
            ("25", FrameRate::new(25, 1)),
        ] {
            assert_eq!(FrameRate::from_text(typed).unwrap(), rate, "{typed}");
        }
        assert!(FrameRate::from_text("fast").is_err());
        assert_eq!(serde_json::to_string(&Time(5)).unwrap(), "5");
    }
}
