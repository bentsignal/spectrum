use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

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
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ShapeGradient {
    pub kind: GradientKind,
    pub angle: f32,
    pub stops: Vec<GradientStop>,
}

impl Default for ShapeGradient {
    fn default() -> Self {
        Self {
            kind: GradientKind::Linear,
            angle: 0.0,
            stops: vec![
                GradientStop::new(0.0, [93, 216, 199, 255]),
                GradientStop::new(1.0, [174, 123, 255, 255]),
            ],
        }
    }
}

impl ShapeGradient {
    pub(crate) fn sanitized(mut self) -> Self {
        self.angle = self.angle.rem_euclid(360.0);
        for stop in &mut self.stops {
            stop.position = stop.position.clamp(0.0, 1.0);
        }
        self.stops
            .sort_by(|left, right| left.position.total_cmp(&right.position));
        self
    }

    fn direction(&self) -> (f32, f32) {
        let radians = self.angle.to_radians();
        (radians.cos(), radians.sin())
    }

    fn sample(&self, normalized_x: f32, normalized_y: f32, direction: (f32, f32)) -> [u8; 4] {
        let Some(start) = self.stops.first().copied() else {
            return [0; 4];
        };
        let Some(end) = self.stops.get(1).copied() else {
            return start.color;
        };
        let amount = gradient_amount(start, end, normalized_x, normalized_y, direction);
        interpolate_premultiplied(start.color, end.color, amount)
    }

    fn sample_alpha(&self, normalized_x: f32, normalized_y: f32, direction: (f32, f32)) -> u8 {
        let Some(start) = self.stops.first().copied() else {
            return 0;
        };
        let Some(end) = self.stops.get(1).copied() else {
            return start.color[3];
        };
        let amount = gradient_amount(start, end, normalized_x, normalized_y, direction);
        (f32::from(start.color[3]) + (f32::from(end.color[3]) - f32::from(start.color[3])) * amount)
            .round()
            .clamp(0.0, 255.0) as u8
    }

    fn uniform_color(&self) -> Option<[u8; 4]> {
        let first = self.stops.first()?;
        self.stops
            .iter()
            .all(|stop| stop.color == first.color)
            .then_some(first.color)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ShapeFill {
    Gradient(ShapeGradient),
}

impl ShapeFill {
    pub(crate) fn sanitized(self) -> Self {
        match self {
            Self::Gradient(gradient) => Self::Gradient(gradient.sanitized()),
        }
    }

    pub(crate) fn direction(&self) -> (f32, f32) {
        match self {
            Self::Gradient(gradient) => gradient.direction(),
        }
    }

    pub(crate) fn sample(
        &self,
        x: f32,
        y: f32,
        width: u32,
        height: u32,
        direction: (f32, f32),
    ) -> [u8; 4] {
        match self {
            Self::Gradient(gradient) => gradient.sample(
                (x / width.max(1) as f32).clamp(0.0, 1.0),
                (y / height.max(1) as f32).clamp(0.0, 1.0),
                direction,
            ),
        }
    }

    pub(crate) fn uniform_color(&self) -> Option<[u8; 4]> {
        match self {
            Self::Gradient(gradient) => gradient.uniform_color(),
        }
    }

    pub(crate) fn sample_alpha(
        &self,
        x: f32,
        y: f32,
        width: u32,
        height: u32,
        direction: (f32, f32),
    ) -> u8 {
        match self {
            Self::Gradient(gradient) => gradient.sample_alpha(
                (x / width.max(1) as f32).clamp(0.0, 1.0),
                (y / height.max(1) as f32).clamp(0.0, 1.0),
                direction,
            ),
        }
    }
}

fn gradient_amount(
    start: GradientStop,
    end: GradientStop,
    normalized_x: f32,
    normalized_y: f32,
    direction: (f32, f32),
) -> f32 {
    let projection =
        ((normalized_x - 0.5) * direction.0 + (normalized_y - 0.5) * direction.1 + 0.5)
            .clamp(0.0, 1.0);
    let span = (end.position - start.position).max(f32::EPSILON);
    ((projection - start.position) / span).clamp(0.0, 1.0)
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
            require_finite("gradient angle", gradient.angle)?;
            if gradient.stops.len() != 2 {
                bail!("this Prism version requires exactly two gradient stops");
            }
            for stop in &gradient.stops {
                require_finite("gradient stop position", stop.position)?;
                if !(0.0..=1.0).contains(&stop.position) {
                    bail!("gradient stop positions must be between 0 and 1");
                }
            }
            if gradient.stops[0].position >= gradient.stops[1].position {
                bail!("gradient stop positions must be strictly increasing");
            }
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
