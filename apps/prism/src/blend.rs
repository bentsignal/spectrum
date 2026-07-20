use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlendMode {
    #[default]
    Normal,
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

impl BlendMode {
    pub const ALL: [Self; 26] = [
        Self::Normal,
        Self::Darken,
        Self::Multiply,
        Self::ColorBurn,
        Self::LinearBurn,
        Self::DarkerColor,
        Self::Lighten,
        Self::Screen,
        Self::ColorDodge,
        Self::LinearDodge,
        Self::LighterColor,
        Self::Overlay,
        Self::SoftLight,
        Self::HardLight,
        Self::VividLight,
        Self::LinearLight,
        Self::PinLight,
        Self::HardMix,
        Self::Difference,
        Self::Exclusion,
        Self::Subtract,
        Self::Divide,
        Self::Hue,
        Self::Saturation,
        Self::Color,
        Self::Luminosity,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Normal => "Normal",
            Self::Darken => "Darken",
            Self::Multiply => "Multiply",
            Self::ColorBurn => "Color Burn",
            Self::LinearBurn => "Linear Burn",
            Self::DarkerColor => "Darker Color",
            Self::Lighten => "Lighten",
            Self::Screen => "Screen",
            Self::ColorDodge => "Color Dodge",
            Self::LinearDodge => "Linear Dodge (Add)",
            Self::LighterColor => "Lighter Color",
            Self::Overlay => "Overlay",
            Self::SoftLight => "Soft Light",
            Self::HardLight => "Hard Light",
            Self::VividLight => "Vivid Light",
            Self::LinearLight => "Linear Light",
            Self::PinLight => "Pin Light",
            Self::HardMix => "Hard Mix",
            Self::Difference => "Difference",
            Self::Exclusion => "Exclusion",
            Self::Subtract => "Subtract",
            Self::Divide => "Divide",
            Self::Hue => "Hue",
            Self::Saturation => "Saturation",
            Self::Color => "Color",
            Self::Luminosity => "Luminosity",
        }
    }

    pub const fn description(self) -> &'static str {
        match self {
            Self::Normal => "Uses the layer color without blending it with layers below.",
            Self::Darken => "Keeps the darker channel value.",
            Self::Multiply => "Darkens by multiplying colors; white has no effect.",
            Self::ColorBurn => "Darkens the base and increases shadow contrast.",
            Self::LinearBurn => "Darkens by subtracting inverted brightness.",
            Self::DarkerColor => "Keeps the complete color with lower luminosity.",
            Self::Lighten => "Keeps the lighter channel value.",
            Self::Screen => "Lightens like projected light; black has no effect.",
            Self::ColorDodge => "Brightens the base and intensifies highlights.",
            Self::LinearDodge => "Adds channel values for a bright linear result.",
            Self::LighterColor => "Keeps the complete color with higher luminosity.",
            Self::Overlay => "Boosts contrast using the base color as the pivot.",
            Self::SoftLight => "Adds gentle contrast and lighting.",
            Self::HardLight => "Adds strong contrast using the layer as the pivot.",
            Self::VividLight => "Combines Color Burn and Color Dodge.",
            Self::LinearLight => "Combines Linear Burn and Linear Dodge.",
            Self::PinLight => "Replaces colors outside a threshold range.",
            Self::HardMix => "Reduces Vivid Light to hard channel thresholds.",
            Self::Difference => "Shows the absolute channel difference.",
            Self::Exclusion => "Creates a softer, lower-contrast Difference.",
            Self::Subtract => "Subtracts the layer color from the base.",
            Self::Divide => "Divides the base by the layer color.",
            Self::Hue => "Uses layer hue with base saturation and luminosity.",
            Self::Saturation => "Uses layer saturation with base hue and luminosity.",
            Self::Color => "Uses layer hue and saturation with base luminosity.",
            Self::Luminosity => "Uses layer luminosity with base hue and saturation.",
        }
    }
}

pub fn blend_rgb(source: [u8; 4], destination: [u8; 4], mode: BlendMode) -> [u8; 3] {
    let source = rgb_to_unit(source);
    let destination = rgb_to_unit(destination);
    let result = match mode {
        BlendMode::DarkerColor => {
            if channel_total(source) < channel_total(destination) {
                source
            } else {
                destination
            }
        }
        BlendMode::LighterColor => {
            if channel_total(source) > channel_total(destination) {
                source
            } else {
                destination
            }
        }
        BlendMode::Hue => set_lum(
            set_sat(source, saturation(destination)),
            luminosity(destination),
        ),
        BlendMode::Saturation => set_lum(
            set_sat(destination, saturation(source)),
            luminosity(destination),
        ),
        BlendMode::Color => set_lum(source, luminosity(destination)),
        BlendMode::Luminosity => set_lum(destination, luminosity(source)),
        _ => std::array::from_fn(|channel| {
            blend_channel(source[channel], destination[channel], mode)
        }),
    };
    std::array::from_fn(|channel| unit_to_u8(result[channel]))
}

fn blend_channel(source: f32, destination: f32, mode: BlendMode) -> f32 {
    match mode {
        BlendMode::Normal => source,
        BlendMode::Darken => source.min(destination),
        BlendMode::Multiply => source * destination,
        BlendMode::ColorBurn => color_burn(source, destination),
        BlendMode::LinearBurn => (source + destination - 1.0).max(0.0),
        BlendMode::Lighten => source.max(destination),
        BlendMode::Screen => 1.0 - (1.0 - source) * (1.0 - destination),
        BlendMode::ColorDodge => color_dodge(source, destination),
        BlendMode::LinearDodge => (source + destination).min(1.0),
        BlendMode::Overlay => hard_light(destination, source),
        BlendMode::SoftLight => soft_light(source, destination),
        BlendMode::HardLight => hard_light(source, destination),
        BlendMode::VividLight => {
            if source <= 0.5 {
                color_burn(source * 2.0, destination)
            } else {
                color_dodge((source - 0.5) * 2.0, destination)
            }
        }
        BlendMode::LinearLight => (destination + 2.0 * source - 1.0).clamp(0.0, 1.0),
        BlendMode::PinLight => {
            if source <= 0.5 {
                destination.min(source * 2.0)
            } else {
                destination.max(2.0 * source - 1.0)
            }
        }
        BlendMode::HardMix => {
            if blend_channel(source, destination, BlendMode::VividLight) < 0.5 {
                0.0
            } else {
                1.0
            }
        }
        BlendMode::Difference => (destination - source).abs(),
        BlendMode::Exclusion => destination + source - 2.0 * destination * source,
        BlendMode::Subtract => (destination - source).max(0.0),
        BlendMode::Divide => {
            if source <= 0.0 {
                1.0
            } else {
                (destination / source).min(1.0)
            }
        }
        BlendMode::DarkerColor
        | BlendMode::LighterColor
        | BlendMode::Hue
        | BlendMode::Saturation
        | BlendMode::Color
        | BlendMode::Luminosity => unreachable!("non-separable mode handled before channels"),
    }
}

fn color_burn(source: f32, destination: f32) -> f32 {
    if destination >= 1.0 {
        1.0
    } else if source <= 0.0 {
        0.0
    } else {
        1.0 - ((1.0 - destination) / source).min(1.0)
    }
}

fn color_dodge(source: f32, destination: f32) -> f32 {
    if destination <= 0.0 {
        0.0
    } else if source >= 1.0 {
        1.0
    } else {
        (destination / (1.0 - source)).min(1.0)
    }
}

fn hard_light(source: f32, destination: f32) -> f32 {
    if source <= 0.5 {
        2.0 * source * destination
    } else {
        1.0 - 2.0 * (1.0 - source) * (1.0 - destination)
    }
}

fn soft_light(source: f32, destination: f32) -> f32 {
    if source <= 0.5 {
        destination - (1.0 - 2.0 * source) * destination * (1.0 - destination)
    } else {
        let curve = if destination <= 0.25 {
            ((16.0 * destination - 12.0) * destination + 4.0) * destination
        } else {
            destination.sqrt()
        };
        destination + (2.0 * source - 1.0) * (curve - destination)
    }
}

fn rgb_to_unit(color: [u8; 4]) -> [f32; 3] {
    std::array::from_fn(|channel| color[channel] as f32 / 255.0)
}

fn unit_to_u8(value: f32) -> u8 {
    (value * 255.0).round().clamp(0.0, 255.0) as u8
}

fn luminosity(color: [f32; 3]) -> f32 {
    0.3 * color[0] + 0.59 * color[1] + 0.11 * color[2]
}

fn saturation(color: [f32; 3]) -> f32 {
    max_channel(color) - min_channel(color)
}

fn channel_total(color: [f32; 3]) -> f32 {
    color.into_iter().sum()
}

fn set_lum(color: [f32; 3], target: f32) -> [f32; 3] {
    let delta = target - luminosity(color);
    clip_color(color.map(|channel| channel + delta))
}

fn set_sat(mut color: [f32; 3], target: f32) -> [f32; 3] {
    let mut indices = [0, 1, 2];
    indices.sort_by(|left, right| color[*left].total_cmp(&color[*right]));
    let (minimum, middle, maximum) = (indices[0], indices[1], indices[2]);
    if color[maximum] > color[minimum] {
        color[middle] =
            (color[middle] - color[minimum]) * target / (color[maximum] - color[minimum]);
        color[maximum] = target;
    } else {
        color[middle] = 0.0;
        color[maximum] = 0.0;
    }
    color[minimum] = 0.0;
    color
}

fn clip_color(mut color: [f32; 3]) -> [f32; 3] {
    let lum = luminosity(color);
    let min = min_channel(color);
    let max = max_channel(color);
    if min < 0.0 {
        color = color.map(|channel| lum + (channel - lum) * lum / (lum - min));
    }
    if max > 1.0 {
        color = color.map(|channel| lum + (channel - lum) * (1.0 - lum) / (max - lum));
    }
    color
}

fn min_channel(color: [f32; 3]) -> f32 {
    color.into_iter().fold(f32::INFINITY, f32::min)
}

fn max_channel(color: [f32; 3]) -> f32 {
    color.into_iter().fold(f32::NEG_INFINITY, f32::max)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_blend_mode_has_a_stable_representative_pixel() {
        let source = [190, 80, 220, 255];
        let destination = [45, 170, 110, 255];
        let actual: Vec<_> = BlendMode::ALL
            .into_iter()
            .map(|mode| (mode, blend_rgb(source, destination, mode)))
            .collect();
        let expected = vec![
            (BlendMode::Normal, [190, 80, 220]),
            (BlendMode::Darken, [45, 80, 110]),
            (BlendMode::Multiply, [34, 53, 95]),
            (BlendMode::ColorBurn, [0, 0, 87]),
            (BlendMode::LinearBurn, [0, 0, 75]),
            (BlendMode::DarkerColor, [45, 170, 110]),
            (BlendMode::Lighten, [190, 170, 220]),
            (BlendMode::Screen, [201, 197, 235]),
            (BlendMode::ColorDodge, [177, 248, 255]),
            (BlendMode::LinearDodge, [235, 250, 255]),
            (BlendMode::LighterColor, [190, 80, 220]),
            (BlendMode::Overlay, [67, 138, 190]),
            (BlendMode::SoftLight, [75, 149, 152]),
            (BlendMode::HardLight, [148, 107, 215]),
            (BlendMode::VividLight, [88, 120, 255]),
            (BlendMode::LinearLight, [170, 75, 255]),
            (BlendMode::PinLight, [125, 160, 185]),
            (BlendMode::HardMix, [0, 0, 255]),
            (BlendMode::Difference, [145, 90, 110]),
            (BlendMode::Exclusion, [168, 143, 140]),
            (BlendMode::Subtract, [0, 90, 0]),
            (BlendMode::Divide, [60, 255, 128]),
            (BlendMode::Hue, [181, 83, 208]),
            (BlendMode::Saturation, [35, 175, 108]),
            (BlendMode::Color, [188, 77, 218]),
            (BlendMode::Luminosity, [48, 173, 113]),
        ];
        assert_eq!(actual, expected);
    }

    #[test]
    fn burn_and_dodge_preserve_absolute_white_and_black_edges() {
        assert_eq!(
            blend_rgb([0, 0, 0, 255], [255, 255, 255, 255], BlendMode::ColorBurn),
            [255; 3]
        );
        assert_eq!(
            blend_rgb([255, 255, 255, 255], [0, 0, 0, 255], BlendMode::ColorDodge),
            [0; 3]
        );
    }

    #[test]
    fn darker_and_lighter_color_compare_total_channels_not_luminosity() {
        let red = [255, 0, 0, 255];
        let green = [0, 200, 0, 255];
        assert_eq!(blend_rgb(red, green, BlendMode::DarkerColor), [0, 200, 0]);
        assert_eq!(blend_rgb(red, green, BlendMode::LighterColor), [255, 0, 0]);
    }
}
