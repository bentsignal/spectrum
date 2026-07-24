use clap::ValueEnum;
use prism_core::BlendMode;

#[derive(Clone, Copy, ValueEnum)]
pub(super) enum CliBlend {
    Normal,
    Dissolve,
    Darken,
    Multiply,
    ColorBurn,
    LinearBurn,
    DarkerColor,
    Lighten,
    Screen,
    ColorDodge,
    LinearDodge,
    LighterColor,
    Overlay,
    SoftLight,
    HardLight,
    VividLight,
    LinearLight,
    PinLight,
    HardMix,
    Difference,
    Exclusion,
    Subtract,
    Divide,
    Hue,
    Saturation,
    Color,
    Luminosity,
}

impl From<CliBlend> for BlendMode {
    fn from(value: CliBlend) -> Self {
        match value {
            CliBlend::Normal => Self::Normal,
            CliBlend::Dissolve => Self::Dissolve,
            CliBlend::Darken => Self::Darken,
            CliBlend::Multiply => Self::Multiply,
            CliBlend::ColorBurn => Self::ColorBurn,
            CliBlend::LinearBurn => Self::LinearBurn,
            CliBlend::DarkerColor => Self::DarkerColor,
            CliBlend::Lighten => Self::Lighten,
            CliBlend::Screen => Self::Screen,
            CliBlend::ColorDodge => Self::ColorDodge,
            CliBlend::LinearDodge => Self::LinearDodge,
            CliBlend::LighterColor => Self::LighterColor,
            CliBlend::Overlay => Self::Overlay,
            CliBlend::SoftLight => Self::SoftLight,
            CliBlend::HardLight => Self::HardLight,
            CliBlend::VividLight => Self::VividLight,
            CliBlend::LinearLight => Self::LinearLight,
            CliBlend::PinLight => Self::PinLight,
            CliBlend::HardMix => Self::HardMix,
            CliBlend::Difference => Self::Difference,
            CliBlend::Exclusion => Self::Exclusion,
            CliBlend::Subtract => Self::Subtract,
            CliBlend::Divide => Self::Divide,
            CliBlend::Hue => Self::Hue,
            CliBlend::Saturation => Self::Saturation,
            CliBlend::Color => Self::Color,
            CliBlend::Luminosity => Self::Luminosity,
        }
    }
}
