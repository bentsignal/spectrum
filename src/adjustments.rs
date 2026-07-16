use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CropRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Default for CropRect {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
        }
    }
}

impl CropRect {
    pub fn sanitized(mut self) -> Self {
        self.x = self.x.clamp(0.0, 0.99);
        self.y = self.y.clamp(0.0, 0.99);
        self.width = self.width.clamp(0.01, 1.0 - self.x);
        self.height = self.height.clamp(0.01, 1.0 - self.y);
        self
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HslBand {
    pub hue: f32,
    pub saturation: f32,
    pub luminance: f32,
}

impl HslBand {
    fn sanitized(mut self) -> Self {
        self.hue = clamp_percent(self.hue);
        self.saturation = clamp_percent(self.saturation);
        self.luminance = clamp_percent(self.luminance);
        self
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HslAdjustments {
    pub red: HslBand,
    pub orange: HslBand,
    pub yellow: HslBand,
    pub green: HslBand,
    pub aqua: HslBand,
    pub blue: HslBand,
    pub purple: HslBand,
    pub magenta: HslBand,
}

impl HslAdjustments {
    pub fn is_identity(&self) -> bool {
        *self == Self::default()
    }

    pub fn band(&self, index: usize) -> HslBand {
        match index {
            0 => self.red,
            1 => self.orange,
            2 => self.yellow,
            3 => self.green,
            4 => self.aqua,
            5 => self.blue,
            6 => self.purple,
            _ => self.magenta,
        }
    }

    pub fn band_mut(&mut self, index: usize) -> &mut HslBand {
        match index {
            0 => &mut self.red,
            1 => &mut self.orange,
            2 => &mut self.yellow,
            3 => &mut self.green,
            4 => &mut self.aqua,
            5 => &mut self.blue,
            6 => &mut self.purple,
            _ => &mut self.magenta,
        }
    }

    pub fn bands(&self) -> [&HslBand; 8] {
        [
            &self.red,
            &self.orange,
            &self.yellow,
            &self.green,
            &self.aqua,
            &self.blue,
            &self.purple,
            &self.magenta,
        ]
    }

    fn sanitized(mut self) -> Self {
        for index in 0..8 {
            *self.band_mut(index) = self.band(index).sanitized();
        }
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CurvePoint {
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ToneCurve {
    pub points: Vec<CurvePoint>,
}

impl Default for ToneCurve {
    fn default() -> Self {
        Self {
            points: vec![CurvePoint { x: 0.0, y: 0.0 }, CurvePoint { x: 1.0, y: 1.0 }],
        }
    }
}

impl ToneCurve {
    pub fn sanitized(mut self) -> Self {
        self.points.retain(|p| p.x.is_finite() && p.y.is_finite());
        for point in &mut self.points {
            point.x = point.x.clamp(0.0, 1.0);
            point.y = point.y.clamp(0.0, 1.0);
        }
        self.points.sort_by(|a, b| a.x.total_cmp(&b.x));
        self.points.dedup_by(|a, b| (a.x - b.x).abs() < 0.001);
        if self.points.first().is_none_or(|point| point.x > 0.001) {
            self.points.insert(0, CurvePoint { x: 0.0, y: 0.0 });
        } else if let Some(first) = self.points.first_mut() {
            first.x = 0.0;
        }
        if self.points.last().is_none_or(|point| point.x < 0.999) {
            self.points.push(CurvePoint { x: 1.0, y: 1.0 });
        } else if let Some(last) = self.points.last_mut() {
            last.x = 1.0;
        }
        self.points.truncate(32);
        self
    }

    pub fn evaluate(&self, input: f32) -> f32 {
        let input = input.clamp(0.0, 1.0);
        for pair in self.points.windows(2) {
            if input <= pair[1].x {
                let span = (pair[1].x - pair[0].x).max(f32::EPSILON);
                let t = (input - pair[0].x) / span;
                return pair[0].y + (pair[1].y - pair[0].y) * t;
            }
        }
        self.points.last().map_or(input, |point| point.y)
    }

    pub fn is_identity(&self) -> bool {
        self.points.len() == 2
            && self.points[0] == CurvePoint { x: 0.0, y: 0.0 }
            && self.points[1] == CurvePoint { x: 1.0, y: 1.0 }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ToneCurves {
    pub master: ToneCurve,
    pub red: ToneCurve,
    pub green: ToneCurve,
    pub blue: ToneCurve,
}

impl ToneCurves {
    pub fn is_identity(&self) -> bool {
        self.master.is_identity()
            && self.red.is_identity()
            && self.green.is_identity()
            && self.blue.is_identity()
    }
}

/// Nondestructive edit settings shared by the GUI and CLI.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Adjustments {
    pub exposure: f32,
    pub temperature: f32,
    pub tint: f32,
    pub contrast: f32,
    pub highlights: f32,
    pub shadows: f32,
    pub whites: f32,
    pub blacks: f32,
    pub texture: f32,
    pub clarity: f32,
    pub dehaze: f32,
    pub vibrance: f32,
    pub saturation: f32,
    pub vignette: f32,
    pub sharpening: f32,
    pub noise_reduction: f32,
    pub rotation: i32,
    pub flip_horizontal: bool,
    pub flip_vertical: bool,
    pub straighten: f32,
    pub crop: Option<CropRect>,
    pub hsl: HslAdjustments,
    pub curves: ToneCurves,
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
            texture: 0.0,
            clarity: 0.0,
            dehaze: 0.0,
            vibrance: 0.0,
            saturation: 0.0,
            vignette: 0.0,
            sharpening: 0.0,
            noise_reduction: 0.0,
            rotation: 0,
            flip_horizontal: false,
            flip_vertical: false,
            straighten: 0.0,
            crop: None,
            hsl: HslAdjustments::default(),
            curves: ToneCurves::default(),
        }
    }
}

impl Adjustments {
    pub fn sanitized(mut self) -> Self {
        self.exposure = self.exposure.clamp(-5.0, 5.0);
        self.temperature = clamp_percent(self.temperature);
        self.tint = clamp_percent(self.tint);
        self.contrast = clamp_percent(self.contrast);
        self.highlights = clamp_percent(self.highlights);
        self.shadows = clamp_percent(self.shadows);
        self.whites = clamp_percent(self.whites);
        self.blacks = clamp_percent(self.blacks);
        self.texture = clamp_percent(self.texture);
        self.clarity = clamp_percent(self.clarity);
        self.dehaze = clamp_percent(self.dehaze);
        self.vibrance = clamp_percent(self.vibrance);
        self.saturation = clamp_percent(self.saturation);
        self.vignette = clamp_percent(self.vignette);
        self.sharpening = self.sharpening.clamp(0.0, 100.0);
        self.noise_reduction = self.noise_reduction.clamp(0.0, 100.0);
        self.rotation = self.rotation.rem_euclid(360) / 90 * 90;
        self.straighten = self.straighten.clamp(-45.0, 45.0);
        self.crop = self.crop.map(CropRect::sanitized);
        self.hsl = self.hsl.sanitized();
        self.curves.master = self.curves.master.sanitized();
        self.curves.red = self.curves.red.sanitized();
        self.curves.green = self.curves.green.sanitized();
        self.curves.blue = self.curves.blue.sanitized();
        self
    }

    pub fn is_identity(&self) -> bool {
        *self == Self::default()
    }

    /// Copy only reusable development settings. Geometry is intentionally
    /// excluded so applying a look never rotates or crops another photo.
    pub fn as_preset(&self) -> Self {
        Self {
            rotation: 0,
            flip_horizontal: false,
            flip_vertical: false,
            straighten: 0.0,
            crop: None,
            ..self.clone()
        }
    }

    /// Apply reusable development settings while preserving this photo's
    /// crop, straighten, rotation, and flips.
    pub fn apply_preset(&mut self, preset: &Self) {
        let geometry = (
            self.rotation,
            self.flip_horizontal,
            self.flip_vertical,
            self.straighten,
            self.crop,
        );
        *self = preset.as_preset();
        self.rotation = geometry.0;
        self.flip_horizontal = geometry.1;
        self.flip_vertical = geometry.2;
        self.straighten = geometry.3;
        self.crop = geometry.4;
        *self = self.clone().sanitized();
    }
}

fn clamp_percent(value: f32) -> f32 {
    value.clamp(-100.0, 100.0)
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
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
    pub texture: Option<f32>,
    pub clarity: Option<f32>,
    pub dehaze: Option<f32>,
    pub vibrance: Option<f32>,
    pub saturation: Option<f32>,
    pub vignette: Option<f32>,
    pub sharpening: Option<f32>,
    pub noise_reduction: Option<f32>,
    pub rotation: Option<i32>,
    pub flip_horizontal: Option<bool>,
    pub flip_vertical: Option<bool>,
    pub straighten: Option<f32>,
    pub crop: Option<Option<CropRect>>,
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
        apply!(texture);
        apply!(clarity);
        apply!(dehaze);
        apply!(vibrance);
        apply!(saturation);
        apply!(vignette);
        apply!(sharpening);
        apply!(noise_reduction);
        apply!(rotation);
        apply!(flip_horizontal);
        apply!(flip_vertical);
        apply!(straighten);
        apply!(crop);
        *value = value.clone().sanitized();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sparse_patch_preserves_and_clamps() {
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
    fn curve_sorts_clamps_and_interpolates() {
        let curve = ToneCurve {
            points: vec![
                CurvePoint { x: 1.2, y: 2.0 },
                CurvePoint { x: 0.5, y: 0.75 },
            ],
        }
        .sanitized();
        assert_eq!(curve.points.first().unwrap().x, 0.0);
        assert_eq!(curve.points.last().unwrap().x, 1.0);
        assert!((curve.evaluate(0.25) - 0.375).abs() < 0.001);
    }

    #[test]
    fn crop_stays_inside_image() {
        let crop = CropRect {
            x: 0.8,
            y: -1.0,
            width: 0.9,
            height: 0.0,
        }
        .sanitized();
        assert!(crop.x + crop.width <= 1.0);
        assert!(crop.height >= 0.01);
    }

    #[test]
    fn presets_preserve_geometry() {
        let mut target = Adjustments {
            rotation: 90,
            crop: Some(CropRect {
                x: 0.1,
                y: 0.1,
                width: 0.8,
                height: 0.8,
            }),
            ..Default::default()
        };
        target.apply_preset(&Adjustments {
            exposure: 1.25,
            rotation: 180,
            ..Default::default()
        });
        assert_eq!(target.exposure, 1.25);
        assert_eq!(target.rotation, 90);
        assert!(target.crop.is_some());
    }
}
