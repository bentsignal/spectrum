//! Every color slider: its range and how it reads and writes the engine's
//! adjustment model. Mixer and grading sliders act on the chosen band.
use spectrum_image::Adjustments;

/// Color mode sections, one shown at a time.
pub const SECTIONS: [&str; 6] = ["Light", "Color", "Curves", "Mixer", "Grading", "Detail"];
pub const CURVES: usize = 2;
pub const MIXER: usize = 3;
pub const GRADING: usize = 4;

pub const BANDS: [&str; 8] = [
    "Red", "Orange", "Yellow", "Green", "Aqua", "Blue", "Purple", "Magenta",
];
/// Swatches for the mixer's band buttons.
pub const BAND_COLORS: [u32; 8] = [
    0xe5484d, 0xf0883e, 0xe5c34b, 0x4cb571, 0x3fb8b0, 0x4a7fe0, 0x8e5ad6, 0xd15aa8,
];
pub const RANGES: [&str; 3] = ["Shadows", "Midtones", "Highlights"];

#[derive(Clone, Copy, PartialEq)]
pub enum Key {
    Exposure,
    Contrast,
    Highlights,
    Shadows,
    Whites,
    Blacks,
    Temperature,
    Tint,
    Vibrance,
    Saturation,
    Texture,
    Clarity,
    Dehaze,
    Sharpening,
    NoiseReduction,
    Vignette,
    MixHue,
    MixSaturation,
    MixLuminance,
    GradeHue,
    GradeSaturation,
    GradeLuminance,
    Balance,
}

pub struct Field {
    pub key: Key,
    pub label: &'static str,
    pub min: f32,
    pub max: f32,
    pub step: f32,
    pub decimals: usize,
    /// Which section shows this slider.
    pub section: usize,
}

const fn field(key: Key, label: &'static str, section: usize, min: f32, max: f32) -> Field {
    Field {
        key,
        label,
        min,
        max,
        step: 1.,
        decimals: 0,
        section,
    }
}

pub const FIELDS: [Field; 23] = [
    Field {
        key: Key::Exposure,
        label: "Exposure",
        min: -5.,
        max: 5.,
        step: 0.05,
        decimals: 2,
        section: 0,
    },
    field(Key::Contrast, "Contrast", 0, -100., 100.),
    field(Key::Highlights, "Highlights", 0, -100., 100.),
    field(Key::Shadows, "Shadows", 0, -100., 100.),
    field(Key::Whites, "Whites", 0, -100., 100.),
    field(Key::Blacks, "Blacks", 0, -100., 100.),
    field(Key::Temperature, "Temperature", 1, -100., 100.),
    field(Key::Tint, "Tint", 1, -100., 100.),
    field(Key::Vibrance, "Vibrance", 1, -100., 100.),
    field(Key::Saturation, "Saturation", 1, -100., 100.),
    field(Key::MixHue, "Hue", MIXER, -100., 100.),
    field(Key::MixSaturation, "Saturation", MIXER, -100., 100.),
    field(Key::MixLuminance, "Luminance", MIXER, -100., 100.),
    field(Key::GradeHue, "Hue", GRADING, 0., 359.),
    field(Key::GradeSaturation, "Saturation", GRADING, 0., 100.),
    field(Key::GradeLuminance, "Luminance", GRADING, -100., 100.),
    field(Key::Balance, "Balance", GRADING, -100., 100.),
    field(Key::Texture, "Texture", 5, -100., 100.),
    field(Key::Clarity, "Clarity", 5, -100., 100.),
    field(Key::Dehaze, "Dehaze", 5, -100., 100.),
    field(Key::Sharpening, "Sharpening", 5, 0., 100.),
    field(Key::NoiseReduction, "Noise reduction", 5, 0., 100.),
    field(Key::Vignette, "Vignette", 5, -100., 100.),
];

fn grade(a: &mut Adjustments, range: usize) -> &mut spectrum_image::ColorGrade {
    match range {
        0 => &mut a.color_grading.shadows,
        1 => &mut a.color_grading.midtones,
        _ => &mut a.color_grading.highlights,
    }
}

/// Reads a slider's value; `band` and `range` pick the mixer band and grading range.
pub fn get(a: &Adjustments, key: Key, band: usize, range: usize) -> f32 {
    let mut a = a.clone();
    *slot(&mut a, key, band, range)
}

pub fn set(a: &mut Adjustments, key: Key, band: usize, range: usize, value: f32) {
    *slot(a, key, band, range) = value;
}

fn slot(a: &mut Adjustments, key: Key, band: usize, range: usize) -> &mut f32 {
    match key {
        Key::Exposure => &mut a.exposure,
        Key::Contrast => &mut a.contrast,
        Key::Highlights => &mut a.highlights,
        Key::Shadows => &mut a.shadows,
        Key::Whites => &mut a.whites,
        Key::Blacks => &mut a.blacks,
        Key::Temperature => &mut a.temperature,
        Key::Tint => &mut a.tint,
        Key::Vibrance => &mut a.vibrance,
        Key::Saturation => &mut a.saturation,
        Key::Texture => &mut a.texture,
        Key::Clarity => &mut a.clarity,
        Key::Dehaze => &mut a.dehaze,
        Key::Sharpening => &mut a.sharpening,
        Key::NoiseReduction => &mut a.noise_reduction,
        Key::Vignette => &mut a.vignette,
        Key::MixHue => &mut a.hsl.band_mut(band).hue,
        Key::MixSaturation => &mut a.hsl.band_mut(band).saturation,
        Key::MixLuminance => &mut a.hsl.band_mut(band).luminance,
        Key::GradeHue => &mut grade(a, range).hue,
        Key::GradeSaturation => &mut grade(a, range).saturation,
        Key::GradeLuminance => &mut grade(a, range).luminance,
        Key::Balance => &mut a.color_grading.balance,
    }
}

/// Resets the sliders in one section, leaving other sections alone. Mixer
/// and grading reset only the chosen band or range.
pub fn reset_section(a: &mut Adjustments, section: usize, band: usize, range: usize) {
    let defaults = Adjustments::default();
    match section {
        CURVES => a.curves = defaults.curves,
        MIXER => *a.hsl.band_mut(band) = Default::default(),
        GRADING => *grade(a, range) = Default::default(),
        _ => {
            for field in FIELDS.iter().filter(|f| f.section == section) {
                set(
                    a,
                    field.key,
                    band,
                    range,
                    get(&defaults, field.key, band, range),
                );
            }
        }
    }
}
