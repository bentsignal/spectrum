//! Bounded, deterministic gradients shared by Spectrum applications.

use std::fmt;

use serde::{
    Deserialize, Deserializer, Serialize,
    de::{Error as _, SeqAccess, Visitor},
};

/// The durable upper bound for one gradient.
pub const MAX_GRADIENT_STOPS: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct GradientStop {
    pub position: f32,
    pub color: [u8; 4],
}

impl GradientStop {
    pub const fn new(position: f32, color: [u8; 4]) -> Self {
        Self { position, color }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GradientKind {
    #[default]
    Linear,
    Radial,
    Angle,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GradientSpread {
    #[default]
    Pad,
    Repeat,
    Reflect,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Gradient {
    pub kind: GradientKind,
    pub angle: f32,
    #[serde(deserialize_with = "deserialize_stops")]
    pub stops: Vec<GradientStop>,
    #[serde(skip_serializing_if = "is_default_center")]
    pub center: [f32; 2],
    #[serde(skip_serializing_if = "is_default_radius")]
    pub radius: f32,
    #[serde(skip_serializing_if = "is_pad")]
    pub spread: GradientSpread,
}

impl Default for Gradient {
    fn default() -> Self {
        Self {
            kind: GradientKind::Linear,
            angle: 0.0,
            stops: vec![
                GradientStop::new(0.0, [93, 216, 199, 255]),
                GradientStop::new(1.0, [174, 123, 255, 255]),
            ],
            center: default_center(),
            radius: default_radius(),
            spread: GradientSpread::Pad,
        }
    }
}

impl Gradient {
    /// Validates durable input without repairing, sorting, or dropping it.
    pub fn validate(&self) -> Result<(), GradientValidationError> {
        if !self.angle.is_finite() {
            return Err(GradientValidationError::new(
                "gradient angle must be finite",
            ));
        }
        if self
            .center
            .iter()
            .any(|coordinate| !coordinate.is_finite() || !(0.0..=1.0).contains(coordinate))
        {
            return Err(GradientValidationError::new(
                "gradient center coordinates must be finite and between 0 and 1",
            ));
        }
        if !self.radius.is_finite() || self.radius <= 0.0 {
            return Err(GradientValidationError::new(
                "gradient radius must be positive and finite",
            ));
        }
        if !(2..=MAX_GRADIENT_STOPS).contains(&self.stops.len()) {
            return Err(GradientValidationError::new(
                "gradients require between 2 and 32 stops",
            ));
        }
        for stop in &self.stops {
            if !stop.position.is_finite() || !(0.0..=1.0).contains(&stop.position) {
                return Err(GradientValidationError::new(
                    "gradient stop positions must be finite and between 0 and 1",
                ));
            }
        }
        if self
            .stops
            .windows(2)
            .any(|pair| pair[0].position >= pair[1].position)
        {
            return Err(GradientValidationError::new(
                "gradient stop positions must be strictly increasing",
            ));
        }
        Ok(())
    }

    /// Canonicalizes representation only after strict validation succeeds.
    pub fn canonicalized(mut self) -> Self {
        self.angle = self.angle.rem_euclid(360.0);
        self
    }

    /// Whether this gradient needs the modern durable encoding.
    pub fn requires_modern_encoding(&self) -> bool {
        self.kind != GradientKind::Linear
            || self.spread != GradientSpread::Pad
            || self.center != default_center()
            || self.radius != default_radius()
            || self.stops.len() != 2
    }

    pub fn sampler(&self) -> GradientSampler<'_> {
        GradientSampler { gradient: self }
    }

    pub fn uniform_color(&self) -> Option<[u8; 4]> {
        let first = self.stops.first()?;
        self.stops
            .iter()
            .all(|stop| stop.color == first.color)
            .then_some(first.color)
    }
}

#[derive(Clone, Copy)]
pub struct GradientSampler<'a> {
    gradient: &'a Gradient,
}

impl GradientSampler<'_> {
    /// Samples normalized shape coordinates and returns straight RGBA.
    ///
    /// Color and alpha come from the same premultiplied interpolation result.
    pub fn sample(&self, normalized_x: f32, normalized_y: f32) -> [u8; 4] {
        let gradient = self.gradient;
        let raw = match gradient.kind {
            GradientKind::Linear => {
                let radians = gradient.angle.to_radians();
                (normalized_x - 0.5) * radians.cos() + (normalized_y - 0.5) * radians.sin() + 0.5
            }
            GradientKind::Radial => {
                let dx = normalized_x - gradient.center[0];
                let dy = normalized_y - gradient.center[1];
                dx.hypot(dy) / gradient.radius
            }
            GradientKind::Angle => {
                let dx = normalized_x - gradient.center[0];
                let dy = normalized_y - gradient.center[1];
                dy.atan2(dx) / std::f32::consts::TAU + 0.5 - gradient.angle / 360.0
            }
        };
        let position = apply_spread(raw, gradient.spread);
        sample_stops(&gradient.stops, position)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GradientValidationError {
    message: &'static str,
}

impl GradientValidationError {
    const fn new(message: &'static str) -> Self {
        Self { message }
    }
}

impl fmt::Display for GradientValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.message)
    }
}

impl std::error::Error for GradientValidationError {}

fn default_center() -> [f32; 2] {
    [0.5, 0.5]
}

fn default_radius() -> f32 {
    0.5
}

fn is_default_center(value: &[f32; 2]) -> bool {
    *value == default_center()
}

fn is_default_radius(value: &f32) -> bool {
    *value == default_radius()
}

fn is_pad(value: &GradientSpread) -> bool {
    *value == GradientSpread::Pad
}

fn apply_spread(value: f32, spread: GradientSpread) -> f32 {
    match spread {
        GradientSpread::Pad => value.clamp(0.0, 1.0),
        GradientSpread::Repeat => value.rem_euclid(1.0),
        GradientSpread::Reflect => {
            let value = value.rem_euclid(2.0);
            if value <= 1.0 { value } else { 2.0 - value }
        }
    }
}

fn sample_stops(stops: &[GradientStop], position: f32) -> [u8; 4] {
    let Some(first) = stops.first().copied() else {
        return [0; 4];
    };
    if position <= first.position {
        return first.color;
    }
    let index = stops.partition_point(|stop| stop.position < position);
    let Some(end) = stops.get(index).copied() else {
        return stops.last().map_or(first.color, |stop| stop.color);
    };
    let start = stops[index - 1];
    let amount = ((position - start.position) / (end.position - start.position)).clamp(0.0, 1.0);
    interpolate_premultiplied(start.color, end.color, amount)
}

fn interpolate_premultiplied(start: [u8; 4], end: [u8; 4], amount: f32) -> [u8; 4] {
    let start_alpha = f32::from(start[3]) / 255.0;
    let end_alpha = f32::from(end[3]) / 255.0;
    let alpha = start_alpha + (end_alpha - start_alpha) * amount;
    let mut output = [0_u8; 4];
    for channel in 0..3 {
        let start_value = f32::from(start[channel]) * start_alpha;
        let end_value = f32::from(end[channel]) * end_alpha;
        let premultiplied = start_value + (end_value - start_value) * amount;
        output[channel] = if alpha > f32::EPSILON {
            (premultiplied / alpha).round().clamp(0.0, 255.0) as u8
        } else {
            0
        };
    }
    output[3] = (alpha * 255.0).round().clamp(0.0, 255.0) as u8;
    output
}

fn deserialize_stops<'de, D>(deserializer: D) -> Result<Vec<GradientStop>, D::Error>
where
    D: Deserializer<'de>,
{
    struct StopsVisitor;

    impl<'de> Visitor<'de> for StopsVisitor {
        type Value = Vec<GradientStop>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("an array containing at most 32 gradient stops")
        }

        fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
        where
            A: SeqAccess<'de>,
        {
            // A thirty-third slot is the bounded overflow sentinel.
            let mut stops = Vec::with_capacity(MAX_GRADIENT_STOPS + 1);
            while let Some(stop) = sequence.next_element()? {
                stops.push(stop);
                if stops.len() > MAX_GRADIENT_STOPS {
                    return Err(A::Error::custom("gradient contains more than 32 stops"));
                }
            }
            Ok(stops)
        }
    }

    deserializer.deserialize_seq(StopsVisitor)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_serialization_is_the_legacy_three_field_shape() {
        assert_eq!(
            serde_json::to_string(&Gradient::default()).unwrap(),
            r#"{"kind":"linear","angle":0.0,"stops":[{"position":0.0,"color":[93,216,199,255]},{"position":1.0,"color":[174,123,255,255]}]}"#
        );
    }

    #[test]
    fn deserialization_rejects_the_overflow_sentinel() {
        let stops = (0..33)
            .map(|index| {
                format!(
                    r#"{{"position":{},"color":[0,0,0,255]}}"#,
                    index as f32 / 32.0
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        let value = format!(r#"{{"kind":"linear","angle":0,"stops":[{stops}]}}"#);
        assert!(serde_json::from_str::<Gradient>(&value).is_err());
    }

    #[test]
    fn premultiplied_interpolation_uses_one_rgba_result() {
        let gradient = Gradient {
            stops: vec![
                GradientStop::new(0.0, [255, 0, 0, 255]),
                GradientStop::new(1.0, [0, 0, 255, 0]),
            ],
            ..Gradient::default()
        };
        assert_eq!(gradient.sampler().sample(0.5, 0.5), [255, 0, 0, 128]);
    }

    #[test]
    fn legacy_two_stop_linear_pixels_match_the_frozen_projection() {
        let gradient = Gradient {
            angle: 33.0,
            stops: vec![
                GradientStop::new(0.17, [245, 20, 80, 220]),
                GradientStop::new(0.83, [15, 210, 190, 47]),
            ],
            ..Gradient::default()
        };
        let radians = gradient.angle.to_radians();
        let direction = (radians.cos(), radians.sin());
        for y in 0..17 {
            for x in 0..19 {
                let normalized_x = x as f32 / 18.0;
                let normalized_y = y as f32 / 16.0;
                let projection =
                    ((normalized_x - 0.5) * direction.0 + (normalized_y - 0.5) * direction.1 + 0.5)
                        .clamp(0.0, 1.0);
                let amount = ((projection - gradient.stops[0].position)
                    / (gradient.stops[1].position - gradient.stops[0].position))
                    .clamp(0.0, 1.0);
                assert_eq!(
                    gradient.sampler().sample(normalized_x, normalized_y),
                    interpolate_premultiplied(
                        gradient.stops[0].color,
                        gradient.stops[1].color,
                        amount
                    )
                );
            }
        }
    }

    #[test]
    fn validation_never_repairs_unsorted_or_out_of_range_stops() {
        let gradient = Gradient {
            stops: vec![
                GradientStop::new(0.8, [0; 4]),
                GradientStop::new(-0.1, [255; 4]),
            ],
            ..Gradient::default()
        };
        assert!(gradient.validate().is_err());
        let canonical = gradient.clone().canonicalized();
        assert_eq!(canonical.stops, gradient.stops);
    }

    #[test]
    fn spread_modes_are_deterministic() {
        assert_eq!(apply_spread(1.25, GradientSpread::Pad), 1.0);
        assert_eq!(apply_spread(1.25, GradientSpread::Repeat), 0.25);
        assert_eq!(apply_spread(1.25, GradientSpread::Reflect), 0.75);
        assert_eq!(apply_spread(-0.25, GradientSpread::Repeat), 0.75);
        assert_eq!(apply_spread(-0.25, GradientSpread::Reflect), 0.25);
    }
}
