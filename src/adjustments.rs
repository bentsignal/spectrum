use serde::{Deserialize, Serialize};

/// Nondestructive edit settings. Numeric values are deliberately human-readable
/// so catalogs and CLI payloads remain easy for agents to inspect and modify.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Adjustments {
    /// Exposure in photographic stops, from -5 to +5.
    pub exposure: f32,
    /// White balance temperature, from -100 (cool) to +100 (warm).
    pub temperature: f32,
    /// White balance tint, from -100 (green) to +100 (magenta).
    pub tint: f32,
    pub contrast: f32,
    pub highlights: f32,
    pub shadows: f32,
    pub whites: f32,
    pub blacks: f32,
    pub clarity: f32,
    pub vibrance: f32,
    pub saturation: f32,
    pub vignette: f32,
    /// Clockwise rotation in degrees. Normalized to 0, 90, 180, or 270.
    pub rotation: i32,
    pub flip_horizontal: bool,
    pub flip_vertical: bool,
}

impl Default for Adjustments {
    fn default() -> Self {
        Self {
            exposure: 0.0,
            temperature: 0.0,
            tint: 0.0,
            contrast: 0.0,
            highlights: 0.0,
            shadows: 0.0,
            whites: 0.0,
            blacks: 0.0,
            clarity: 0.0,
            vibrance: 0.0,
            saturation: 0.0,
            vignette: 0.0,
            rotation: 0,
            flip_horizontal: false,
            flip_vertical: false,
        }
    }
}

impl Adjustments {
    pub fn sanitized(mut self) -> Self {
        self.exposure = self.exposure.clamp(-5.0, 5.0);
        self.temperature = self.temperature.clamp(-100.0, 100.0);
        self.tint = self.tint.clamp(-100.0, 100.0);
        self.contrast = clamp_percent(self.contrast);
        self.highlights = clamp_percent(self.highlights);
        self.shadows = clamp_percent(self.shadows);
        self.whites = clamp_percent(self.whites);
        self.blacks = clamp_percent(self.blacks);
        self.clarity = clamp_percent(self.clarity);
        self.vibrance = clamp_percent(self.vibrance);
        self.saturation = clamp_percent(self.saturation);
        self.vignette = clamp_percent(self.vignette);
        self.rotation = self.rotation.rem_euclid(360) / 90 * 90;
        self
    }

    pub fn is_identity(&self) -> bool {
        *self == Self::default()
    }
}

fn clamp_percent(value: f32) -> f32 {
    value.clamp(-100.0, 100.0)
}

/// A sparse adjustment update. `None` means "leave the current value alone".
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AdjustmentPatch {
    pub exposure: Option<f32>,
    pub temperature: Option<f32>,
    pub tint: Option<f32>,
    pub contrast: Option<f32>,
    pub highlights: Option<f32>,
    pub shadows: Option<f32>,
    pub whites: Option<f32>,
    pub blacks: Option<f32>,
    pub clarity: Option<f32>,
    pub vibrance: Option<f32>,
    pub saturation: Option<f32>,
    pub vignette: Option<f32>,
    pub rotation: Option<i32>,
    pub flip_horizontal: Option<bool>,
    pub flip_vertical: Option<bool>,
}

impl AdjustmentPatch {
    pub fn apply_to(self, value: &mut Adjustments) {
        macro_rules! apply {
            ($field:ident) => {
                if let Some(next) = self.$field {
                    value.$field = next;
                }
            };
        }

        apply!(exposure);
        apply!(temperature);
        apply!(tint);
        apply!(contrast);
        apply!(highlights);
        apply!(shadows);
        apply!(whites);
        apply!(blacks);
        apply!(clarity);
        apply!(vibrance);
        apply!(saturation);
        apply!(vignette);
        apply!(rotation);
        apply!(flip_horizontal);
        apply!(flip_vertical);
        *value = value.sanitized();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sparse_patch_preserves_other_values_and_clamps() {
        let mut value = Adjustments {
            contrast: 24.0,
            ..Default::default()
        };
        AdjustmentPatch {
            exposure: Some(99.0),
            ..Default::default()
        }
        .apply_to(&mut value);
        assert_eq!(value.exposure, 5.0);
        assert_eq!(value.contrast, 24.0);
    }

    #[test]
    fn rotation_is_normalized() {
        let value = Adjustments {
            rotation: -90,
            ..Default::default()
        }
        .sanitized();
        assert_eq!(value.rotation, 270);
    }
}
