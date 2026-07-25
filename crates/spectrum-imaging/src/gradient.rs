//! Bounded, deterministic gradients shared by Spectrum applications.

use std::fmt;

use serde::{
    Deserialize, Deserializer, Serialize, Serializer,
    de::{Error as _, SeqAccess, Visitor},
    ser::SerializeStruct,
};

/// The durable upper bound for one gradient.
pub const MAX_GRADIENT_STOPS: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GradientInterpolation {
    #[default]
    PremultipliedSrgbV1,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Gradient {
    pub kind: GradientKind,
    pub angle: f32,
    pub stops: Vec<GradientStop>,
    pub center: [f32; 2],
    pub radius: f32,
    pub spread: GradientSpread,
    pub interpolation: GradientInterpolation,
    pub offset: f32,
    pub extent: f32,
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
            interpolation: GradientInterpolation::PremultipliedSrgbV1,
            offset: 0.0,
            extent: 1.0,
        }
    }
}

impl Serialize for Gradient {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let modern = self.requires_modern_encoding();
        let mut state = serializer.serialize_struct("Gradient", 9)?;
        state.serialize_field("kind", &self.kind)?;
        state.serialize_field("angle", &self.angle)?;
        state.serialize_field("stops", &self.stops)?;
        if !is_default_center(&self.center) {
            state.serialize_field("center", &self.center)?;
        }
        if !is_default_radius(&self.radius) {
            state.serialize_field("radius", &self.radius)?;
        }
        if !is_pad(&self.spread) {
            state.serialize_field("spread", &self.spread)?;
        }
        if modern {
            state.serialize_field("interpolation", &self.interpolation)?;
        }
        if !is_zero(&self.offset) {
            state.serialize_field("offset", &self.offset)?;
        }
        if !is_one(&self.extent) {
            state.serialize_field("extent", &self.extent)?;
        }
        state.end()
    }
}

#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
struct GradientWire {
    kind: GradientKind,
    angle: f32,
    #[serde(deserialize_with = "deserialize_stops")]
    stops: Vec<GradientStop>,
    center: [f32; 2],
    radius: f32,
    spread: GradientSpread,
    interpolation: Option<GradientInterpolation>,
    offset: f32,
    extent: f32,
}

impl Default for GradientWire {
    fn default() -> Self {
        let gradient = Gradient::default();
        Self {
            kind: gradient.kind,
            angle: gradient.angle,
            stops: gradient.stops,
            center: gradient.center,
            radius: gradient.radius,
            spread: gradient.spread,
            interpolation: None,
            offset: gradient.offset,
            extent: gradient.extent,
        }
    }
}

impl<'de> Deserialize<'de> for Gradient {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = GradientWire::deserialize(deserializer)?;
        let explicit_interpolation = wire.interpolation.is_some();
        let gradient = Self {
            kind: wire.kind,
            angle: wire.angle,
            stops: wire.stops,
            center: wire.center,
            radius: wire.radius,
            spread: wire.spread,
            interpolation: wire.interpolation.unwrap_or_default(),
            offset: wire.offset,
            extent: wire.extent,
        };
        if gradient.requires_modern_encoding() && !explicit_interpolation {
            return Err(D::Error::custom(
                "modern gradients require an explicit interpolation contract",
            ));
        }
        Ok(gradient)
    }
}

impl Gradient {
    /// Validates durable input without repairing, sorting, or dropping it.
    pub fn validate(&self) -> Result<(), GradientValidationError> {
        match self.interpolation {
            GradientInterpolation::PremultipliedSrgbV1 => {}
        }
        if !self.angle.is_finite() {
            return Err(GradientValidationError::new(
                "gradient angle must be finite",
            ));
        }
        if !self.offset.is_finite() {
            return Err(GradientValidationError::new(
                "gradient offset must be finite",
            ));
        }
        if !self.extent.is_normal() || self.extent <= 0.0 {
            return Err(GradientValidationError::new(
                "gradient extent must be positive, finite, and non-subnormal",
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
        if !self.radius.is_normal() || self.radius <= 0.0 {
            return Err(GradientValidationError::new(
                "gradient radius must be positive, finite, and non-subnormal",
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
        let modern = self.requires_modern_encoding();
        self.angle = self.angle.rem_euclid(360.0);
        if modern {
            for stop in &mut self.stops {
                if stop.color[3] == 0 {
                    stop.color = [0; 4];
                }
            }
        }
        self
    }

    /// Whether this gradient needs the modern durable encoding.
    pub fn requires_modern_encoding(&self) -> bool {
        self.kind != GradientKind::Linear
            || self.spread != GradientSpread::Pad
            || self.center != default_center()
            || self.radius != default_radius()
            || self.stops.len() != 2
            || self.offset != 0.0
            || self.extent != 1.0
    }

    pub fn sampler(&self) -> GradientSampler<'_> {
        GradientSampler {
            gradient: self,
            linear_direction: linear_direction(self.angle),
            uniform_alpha: self.uniform_alpha(),
        }
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
    linear_direction: [f32; 2],
    uniform_alpha: Option<u8>,
}

#[path = "gradient_alpha.rs"]
mod alpha;

/// Logical work performed by explicitly instrumented gradient samples.
///
/// Strict benchmarks use this opt-in path so normal rendering pays no
/// synchronization or counter-update cost.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GradientSampleStats {
    pub samples: u64,
    pub stop_comparisons: u64,
    pub temporary_bytes: u64,
    pub source_copy_bytes: u64,
}

impl GradientSampler<'_> {
    /// Samples normalized shape coordinates and returns straight RGBA.
    ///
    /// Color and alpha come from the same premultiplied interpolation result.
    pub fn sample(&self, normalized_x: f32, normalized_y: f32) -> [u8; 4] {
        self.sample_in_box(normalized_x, normalized_y, 1.0, 1.0)
    }

    /// Samples source-local coordinates in a rectangular shape.
    ///
    /// Linear gradients retain their normalized-box projection for legacy
    /// parity. Radial and Angle gradients use source-pixel distances so a
    /// non-square shape cannot stretch circles or rotate angle directions.
    pub fn sample_in_box(
        &self,
        source_x: f32,
        source_y: f32,
        source_width: f32,
        source_height: f32,
    ) -> [u8; 4] {
        let position = apply_spread(
            self.project_position(source_x, source_y, source_width, source_height),
            self.gradient.spread,
        );
        sample_stops(&self.gradient.stops, position)
    }

    /// Samples while recording the exact binary-search and allocation
    /// contract used by strict performance benchmarks.
    pub fn sample_in_box_with_stats(
        &self,
        source_x: f32,
        source_y: f32,
        source_width: f32,
        source_height: f32,
        stats: &mut GradientSampleStats,
    ) -> [u8; 4] {
        stats.samples = stats.samples.saturating_add(1);
        let position = apply_spread(
            self.project_position(source_x, source_y, source_width, source_height),
            self.gradient.spread,
        );
        sample_stops_impl(&self.gradient.stops, position, || {
            stats.stop_comparisons = stats.stop_comparisons.saturating_add(1);
        })
    }

    fn project_position(
        &self,
        source_x: f32,
        source_y: f32,
        source_width: f32,
        source_height: f32,
    ) -> f32 {
        let gradient = self.gradient;
        match gradient.kind {
            GradientKind::Linear => {
                let normalized_x = source_x / source_width.max(f32::MIN_POSITIVE);
                let normalized_y = source_y / source_height.max(f32::MIN_POSITIVE);
                ((normalized_x - 0.5) * self.linear_direction[0]
                    + (normalized_y - 0.5) * self.linear_direction[1]
                    + 0.5
                    - gradient.offset)
                    / gradient.extent
            }
            GradientKind::Radial => {
                let metric = source_width.min(source_height).max(f32::MIN_POSITIVE);
                let dx = (source_x - gradient.center[0] * source_width) / metric;
                let dy = (source_y - gradient.center[1] * source_height) / metric;
                dx.hypot(dy) / gradient.radius
            }
            GradientKind::Angle => {
                let dx = source_x - gradient.center[0] * source_width;
                let dy = source_y - gradient.center[1] * source_height;
                (angle_turn(dx, dy) - gradient.offset) / gradient.extent - gradient.angle / 360.0
            }
        }
    }

    /// Samples an already projected scalar using the exact render
    /// interpolation contract.
    pub fn sample_position(&self, position: f32) -> [u8; 4] {
        sample_stops(&self.gradient.stops, position)
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

fn is_zero(value: &f32) -> bool {
    *value == 0.0
}

fn is_one(value: &f32) -> bool {
    *value == 1.0
}

fn linear_direction(angle: f32) -> [f32; 2] {
    match angle {
        0.0 | 360.0 => [1.0, 0.0],
        90.0 => [0.0, 1.0],
        180.0 => [-1.0, 0.0],
        270.0 => [0.0, -1.0],
        _ => {
            let radians = angle.to_radians();
            [radians.cos(), radians.sin()]
        }
    }
}

fn angle_turn(dx: f32, dy: f32) -> f32 {
    if dy == 0.0 {
        if dx < 0.0 { 0.5 } else { 0.0 }
    } else if dx == 0.0 {
        if dy > 0.0 { 0.25 } else { 0.75 }
    } else {
        (dy.atan2(dx) / std::f32::consts::TAU).rem_euclid(1.0)
    }
}

fn apply_spread(value: f32, spread: GradientSpread) -> f32 {
    match spread {
        GradientSpread::Pad => value.clamp(0.0, 1.0),
        GradientSpread::Repeat => {
            if value.is_finite() {
                value.rem_euclid(1.0)
            } else {
                0.0
            }
        }
        GradientSpread::Reflect => {
            if value.is_finite() {
                let value = value.rem_euclid(2.0);
                if value <= 1.0 { value } else { 2.0 - value }
            } else {
                0.0
            }
        }
    }
}

fn sample_stops(stops: &[GradientStop], position: f32) -> [u8; 4] {
    sample_stops_impl(stops, position, || {})
}

fn sample_stops_impl(
    stops: &[GradientStop],
    position: f32,
    mut comparison: impl FnMut(),
) -> [u8; 4] {
    let Some(first) = stops.first().copied() else {
        return [0; 4];
    };
    if !position.is_finite() {
        return first.color;
    }
    if position <= first.position {
        return first.color;
    }
    let mut start_index = 0;
    let mut end_index = stops.len();
    while start_index < end_index {
        let middle = start_index + (end_index - start_index) / 2;
        comparison();
        if stops[middle].position < position {
            start_index = middle + 1;
        } else {
            end_index = middle;
        }
    }
    let index = start_index;
    if index == 0 {
        return first.color;
    }
    let Some(end) = stops.get(index).copied() else {
        return stops.last().map_or(first.color, |stop| stop.color);
    };
    let start = stops[index - 1];
    let span = end.position - start.position;
    if !span.is_finite() || span <= 0.0 {
        return start.color;
    }
    let amount = (position - start.position) / span;
    if !amount.is_finite() {
        return start.color;
    }
    let amount = amount.clamp(0.0, 1.0);
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
    fn modern_serialization_requires_an_explicit_interpolation_contract() {
        let modern = Gradient {
            kind: GradientKind::Radial,
            ..Default::default()
        };
        let encoded = serde_json::to_string(&modern).unwrap();
        assert!(encoded.contains(r#""interpolation":"premultiplied_srgb_v1""#));
        assert_eq!(serde_json::from_str::<Gradient>(&encoded).unwrap(), modern);
        assert!(
            serde_json::from_str::<Gradient>(
                r#"{"kind":"radial","angle":0,"stops":[{"position":0,"color":[0,0,0,255]},{"position":1,"color":[255,255,255,255]}]}"#
            )
            .is_err()
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
    fn strict_serde_rejects_unknown_duplicate_and_unsupported_modern_fields() {
        for value in [
            r#"{"kind":"radial","angle":0,"raduis":0.9,"stops":[{"position":0,"color":[0,0,0,255]},{"position":1,"color":[255,255,255,255]}]}"#,
            r#"{"kind":"radial","kind":"angle","angle":0,"stops":[{"position":0,"color":[0,0,0,255]},{"position":1,"color":[255,255,255,255]}]}"#,
            r#"{"kind":"radial","angle":0,"interpolation":"linear_rgb_v1","stops":[{"position":0,"color":[0,0,0,255]},{"position":1,"color":[255,255,255,255]}]}"#,
            r#"{"kind":"radial","angle":0,"stops":[{"position":0,"opacity":1,"color":[0,0,0,255]},{"position":1,"color":[255,255,255,255]}]}"#,
            r#"{"kind":"radial","angle":0,"stops":[{"position":0,"position":0.5,"color":[0,0,0,255]},{"position":1,"color":[255,255,255,255]}]}"#,
        ] {
            assert!(
                serde_json::from_str::<Gradient>(value).is_err(),
                "strict gradient serde accepted {value}"
            );
        }
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
    fn instrumented_sampling_pins_binary_search_and_zero_copy_bounds() {
        let gradient = Gradient {
            kind: GradientKind::Radial,
            stops: (0..MAX_GRADIENT_STOPS)
                .map(|index| {
                    GradientStop::new(
                        index as f32 / (MAX_GRADIENT_STOPS - 1) as f32,
                        [index as u8; 4],
                    )
                })
                .collect(),
            ..Default::default()
        };
        let mut stats = GradientSampleStats::default();
        let sampler = gradient.sampler();
        for y in 0..17 {
            for x in 0..29 {
                sampler.sample_in_box_with_stats(
                    x as f32 + 0.5,
                    y as f32 + 0.5,
                    29.0,
                    17.0,
                    &mut stats,
                );
            }
        }
        assert_eq!(stats.samples, 29 * 17);
        assert!(stats.stop_comparisons <= stats.samples * 6);
        assert_eq!(stats.temporary_bytes, 0);
        assert_eq!(stats.source_copy_bytes, 0);
    }

    #[test]
    fn angle_zero_has_a_positive_x_seam_and_exact_center_zero() {
        let gradient = Gradient {
            kind: GradientKind::Angle,
            stops: vec![
                GradientStop::new(0.0, [0, 0, 0, 255]),
                GradientStop::new(1.0, [255, 255, 255, 255]),
            ],
            ..Default::default()
        };
        let sampler = gradient.sampler();
        assert_eq!(
            sampler.sample_in_box(150.0, 50.0, 200.0, 100.0),
            [0, 0, 0, 255]
        );
        assert_eq!(
            sampler.sample_in_box(100.0, 50.0, 200.0, 100.0),
            [0, 0, 0, 255]
        );
        assert_eq!(
            sampler.sample_in_box(100.0, 75.0, 200.0, 100.0),
            [64, 64, 64, 255]
        );
        assert_eq!(
            sampler.sample_in_box(50.0, 50.0, 200.0, 100.0),
            [128, 128, 128, 255]
        );
        assert_eq!(
            sampler.sample_in_box(100.0, 25.0, 200.0, 100.0),
            [191, 191, 191, 255]
        );
        assert_eq!(
            sampler.sample_in_box(150.0, f32::from_bits(50.0_f32.to_bits() - 1), 200.0, 100.0),
            [255, 255, 255, 255]
        );
    }

    #[test]
    fn angle_phase_and_spreads_have_pinned_wrap_bytes() {
        let base = Gradient {
            kind: GradientKind::Angle,
            angle: 90.0,
            stops: vec![
                GradientStop::new(0.0, [0, 0, 0, 255]),
                GradientStop::new(1.0, [255, 255, 255, 255]),
            ],
            ..Default::default()
        };
        for (spread, expected) in [
            (GradientSpread::Pad, [0, 0, 0, 255]),
            (GradientSpread::Repeat, [191, 191, 191, 255]),
            (GradientSpread::Reflect, [64, 64, 64, 255]),
        ] {
            let gradient = Gradient {
                spread,
                ..base.clone()
            };
            assert_eq!(
                gradient.sampler().sample_in_box(150.0, 50.0, 200.0, 100.0),
                expected
            );
        }
    }

    #[test]
    fn cardinal_linear_directions_have_pinned_exact_bytes() {
        let base = Gradient {
            stops: vec![
                GradientStop::new(0.0, [0, 0, 0, 255]),
                GradientStop::new(1.0, [255, 255, 255, 255]),
            ],
            ..Default::default()
        };
        for (angle, point, expected) in [
            (0.0, (1.0, 0.5), [255, 255, 255, 255]),
            (90.0, (0.5, 1.0), [255, 255, 255, 255]),
            (180.0, (1.0, 0.5), [0, 0, 0, 255]),
            (270.0, (0.5, 1.0), [0, 0, 0, 255]),
        ] {
            let gradient = Gradient {
                angle,
                ..base.clone()
            };
            assert_eq!(gradient.sampler().sample(point.0, point.1), expected);
        }
    }

    #[test]
    fn modern_transparent_rgb_is_canonical_but_legacy_bytes_are_grandfathered() {
        let legacy = Gradient {
            stops: vec![
                GradientStop::new(0.0, [255, 0, 0, 255]),
                GradientStop::new(1.0, [17, 33, 91, 0]),
            ],
            ..Default::default()
        };
        assert_eq!(
            legacy.clone().canonicalized().stops[1].color,
            [17, 33, 91, 0]
        );
        let modern = Gradient {
            kind: GradientKind::Radial,
            ..legacy
        };
        assert_eq!(modern.canonicalized().stops[1].color, [0; 4]);
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
    fn radial_validation_rejects_every_subnormal_radius_but_accepts_safe_extremes() {
        for radius in [
            0.0,
            -0.0,
            f32::from_bits(1),
            f32::MIN_POSITIVE / 2.0,
            -f32::MIN_POSITIVE,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NAN,
        ] {
            let gradient = Gradient {
                kind: GradientKind::Radial,
                radius,
                ..Gradient::default()
            };
            assert!(
                gradient.validate().is_err(),
                "radius {radius:?} was accepted"
            );
        }

        for radius in [f32::MIN_POSITIVE, 0.5, f32::MAX] {
            let gradient = Gradient {
                kind: GradientKind::Radial,
                radius,
                ..Gradient::default()
            };
            gradient.validate().unwrap();
            for spread in [
                GradientSpread::Pad,
                GradientSpread::Repeat,
                GradientSpread::Reflect,
            ] {
                let mut gradient = gradient.clone();
                gradient.spread = spread;
                for (x, y) in [(0.0, 0.0), (0.5, 0.5), (1.0, 1.0)] {
                    let sampled = gradient.sampler().sample(x, y);
                    assert_eq!(sampled, gradient.sampler().sample(x, y));
                }
            }
        }
    }

    #[test]
    fn finite_huge_angles_canonicalize_and_sample_every_geometry_and_spread() {
        for angle in [f32::MAX, -f32::MAX] {
            for kind in [
                GradientKind::Linear,
                GradientKind::Radial,
                GradientKind::Angle,
            ] {
                for spread in [
                    GradientSpread::Pad,
                    GradientSpread::Repeat,
                    GradientSpread::Reflect,
                ] {
                    let gradient = Gradient {
                        kind,
                        angle,
                        spread,
                        radius: f32::MAX,
                        ..Gradient::default()
                    };
                    gradient.validate().unwrap();
                    assert!(gradient.clone().canonicalized().angle.is_finite());
                    for (x, y) in [(0.0, 0.0), (0.25, 0.75), (1.0, 1.0)] {
                        let sampled = gradient.sampler().sample(x, y);
                        assert_eq!(sampled, gradient.sampler().sample(x, y));
                    }
                }
            }
        }
    }

    #[test]
    fn sampler_is_total_for_nonfinite_geometry_inputs_and_malformed_stops() {
        let boundary_values = [
            f32::NEG_INFINITY,
            -f32::MAX,
            -1.0,
            -0.0,
            0.0,
            f32::from_bits(1),
            f32::MIN_POSITIVE,
            0.5,
            1.0,
            f32::MAX,
            f32::INFINITY,
            f32::NAN,
        ];
        for kind in [
            GradientKind::Linear,
            GradientKind::Radial,
            GradientKind::Angle,
        ] {
            for spread in [
                GradientSpread::Pad,
                GradientSpread::Repeat,
                GradientSpread::Reflect,
            ] {
                for value in boundary_values {
                    let gradient = Gradient {
                        kind,
                        spread,
                        angle: value,
                        radius: value,
                        ..Gradient::default()
                    };
                    let sample = gradient.sampler().sample(value, -value);
                    assert_eq!(sample, gradient.sampler().sample(value, -value));
                }
            }
        }

        assert_eq!(sample_stops(&[], f32::NAN), [0; 4]);
        let malformed = [
            GradientStop::new(f32::NAN, [1, 2, 3, 4]),
            GradientStop::new(f32::NEG_INFINITY, [5, 6, 7, 8]),
        ];
        assert_eq!(sample_stops(&malformed, f32::NAN), malformed[0].color);
        let malformed_sample = sample_stops(&malformed, 0.5);
        assert_eq!(malformed_sample, sample_stops(&malformed, 0.5));
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
