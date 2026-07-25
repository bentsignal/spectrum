use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
pub use spectrum_imaging::{
    Gradient as ShapeGradient, GradientKind, GradientSpread, GradientStop, MAX_GRADIENT_STOPS,
};

use crate::validation::require_finite;

pub const MAX_DROP_SHADOW_BLUR: f32 = 128.0;
pub const MAX_DROP_SHADOW_OFFSET: f32 = 4_096.0;
#[doc(hidden)]
pub const DROP_SHADOW_KERNEL: [(f32, f32, u32); 13] = [
    (0.0, 0.0, 4),
    (-0.5, 0.0, 2),
    (0.5, 0.0, 2),
    (0.0, -0.5, 2),
    (0.0, 0.5, 2),
    (-0.5, -0.5, 1),
    (0.5, -0.5, 1),
    (-0.5, 0.5, 1),
    (0.5, 0.5, 1),
    (-1.0, 0.0, 1),
    (1.0, 0.0, 1),
    (0.0, -1.0, 1),
    (0.0, 1.0, 1),
];
pub(crate) const DROP_SHADOW_KERNEL_TAPS: u64 = DROP_SHADOW_KERNEL.len() as u64;

const fn kernel_total_weight() -> u32 {
    let mut index = 0;
    let mut total = 0;
    while index < DROP_SHADOW_KERNEL.len() {
        total += DROP_SHADOW_KERNEL[index].2;
        index += 1;
    }
    total
}

#[doc(hidden)]
pub const DROP_SHADOW_KERNEL_TOTAL_WEIGHT: u32 = kernel_total_weight();

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ShapeStroke {
    pub enabled: bool,
    pub width: f32,
    pub color: [u8; 4],
}

impl Default for ShapeStroke {
    fn default() -> Self {
        Self {
            enabled: false,
            width: 4.0,
            color: [255, 255, 255, 255],
        }
    }
}

impl ShapeStroke {
    pub(crate) fn sanitized(self) -> Self {
        Self {
            enabled: self.enabled,
            width: self.width.clamp(0.5, 512.0),
            color: self.color,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DropShadow {
    pub color: [u8; 4],
    pub offset_x: f32,
    pub offset_y: f32,
    pub blur_radius: f32,
}

impl Default for DropShadow {
    fn default() -> Self {
        Self {
            color: [0, 0, 0, 160],
            offset_x: 12.0,
            offset_y: 12.0,
            blur_radius: 10.0,
        }
    }
}

impl DropShadow {
    pub(crate) fn sanitized(self) -> Self {
        Self {
            color: self.color,
            offset_x: self
                .offset_x
                .clamp(-MAX_DROP_SHADOW_OFFSET, MAX_DROP_SHADOW_OFFSET),
            offset_y: self
                .offset_y
                .clamp(-MAX_DROP_SHADOW_OFFSET, MAX_DROP_SHADOW_OFFSET),
            blur_radius: self.blur_radius.clamp(0.0, MAX_DROP_SHADOW_BLUR),
        }
    }

    pub(crate) fn scaled(self, scale: f32) -> Self {
        Self {
            offset_x: self.offset_x * scale,
            offset_y: self.offset_y * scale,
            blur_radius: self.blur_radius * scale,
            ..self
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LayerStyle {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub drop_shadow: Option<DropShadow>,
}

impl LayerStyle {
    pub fn is_empty(&self) -> bool {
        self.drop_shadow.is_none()
    }

    pub(crate) fn sanitized(self) -> Self {
        Self {
            drop_shadow: self.drop_shadow.map(DropShadow::sanitized),
        }
    }

    pub(crate) fn scaled(&self, scale: f32) -> Self {
        Self {
            drop_shadow: self.drop_shadow.map(|shadow| shadow.scaled(scale)),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ShapeFill {
    Gradient(ShapeGradient),
}

impl ShapeFill {
    pub(crate) fn requires_modern_encoding(&self) -> bool {
        match self {
            Self::Gradient(gradient) => gradient.requires_modern_encoding(),
        }
    }

    pub(crate) fn sanitized(self) -> Self {
        match self {
            Self::Gradient(gradient) => Self::Gradient(gradient.canonicalized()),
        }
    }

    pub(crate) fn sample(&self, x: f32, y: f32, width: u32, height: u32) -> [u8; 4] {
        match self {
            Self::Gradient(gradient) => gradient.sampler().sample(
                (x / width.max(1) as f32).clamp(0.0, 1.0),
                (y / height.max(1) as f32).clamp(0.0, 1.0),
            ),
        }
    }

    pub(crate) fn uniform_color(&self) -> Option<[u8; 4]> {
        match self {
            Self::Gradient(gradient) => gradient.uniform_color(),
        }
    }

    pub(crate) fn sample_alpha(&self, x: f32, y: f32, width: u32, height: u32) -> u8 {
        self.sample(x, y, width, height)[3]
    }
}

pub(crate) fn validate_layer_style(style: &LayerStyle) -> Result<()> {
    let Some(shadow) = style.drop_shadow else {
        return Ok(());
    };
    require_finite("drop shadow horizontal offset", shadow.offset_x)?;
    require_finite("drop shadow vertical offset", shadow.offset_y)?;
    require_finite("drop shadow blur radius", shadow.blur_radius)?;
    if shadow.blur_radius < 0.0 {
        bail!("drop shadow blur radius cannot be negative");
    }
    Ok(())
}

pub(crate) fn validate_shape_fill(fill: &ShapeFill) -> Result<()> {
    match fill {
        ShapeFill::Gradient(gradient) => {
            gradient.validate().map_err(anyhow::Error::new)?;
        }
    }
    Ok(())
}

pub(crate) fn drop_shadow_alpha(
    center_x: i64,
    center_y: i64,
    radius: f32,
    mut alpha_at: impl FnMut(i64, i64) -> u8,
) -> u8 {
    if radius < 0.5 {
        return alpha_at(center_x, center_y);
    }
    let mut weighted_alpha = 0_u32;
    let mut total_weight = 0_u32;
    for (unit_x, unit_y, weight) in DROP_SHADOW_KERNEL {
        let x = center_x + (unit_x * radius).round() as i64;
        let y = center_y + (unit_y * radius).round() as i64;
        weighted_alpha += u32::from(alpha_at(x, y)) * weight;
        total_weight += weight;
    }
    (weighted_alpha / total_weight) as u8
}

pub(crate) fn colored_shadow_pixel(shadow: DropShadow, source_alpha: u8) -> [u8; 4] {
    let alpha = u16::from(source_alpha) * u16::from(shadow.color[3]) / 255;
    [
        shadow.color[0],
        shadow.color[1],
        shadow.color[2],
        alpha as u8,
    ]
}
