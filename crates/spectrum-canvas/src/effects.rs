use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
pub use spectrum_imaging::{
    Gradient as ShapeGradient, GradientInterpolation, GradientKind, GradientSampler,
    GradientSpread, GradientStop, MAX_GRADIENT_STOPS,
};

use crate::{
    layer_effects::{ColorOverlay, Glow, LayerStroke},
    layer_shading::{BevelEmboss, GradientOverlay, Satin},
    validation::require_finite,
};

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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outer_glow: Option<Glow>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color_overlay: Option<ColorOverlay>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inner_glow: Option<Glow>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inner_shadow: Option<DropShadow>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stroke: Option<LayerStroke>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bevel: Option<BevelEmboss>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub satin: Option<Satin>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gradient_overlay: Option<GradientOverlay>,
}

impl LayerStyle {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    pub(crate) fn sanitized(self) -> Self {
        Self {
            drop_shadow: self.drop_shadow.map(DropShadow::sanitized),
            outer_glow: self.outer_glow.map(Glow::sanitized),
            color_overlay: self.color_overlay,
            inner_glow: self.inner_glow.map(Glow::sanitized),
            inner_shadow: self.inner_shadow.map(DropShadow::sanitized),
            stroke: self.stroke.map(LayerStroke::sanitized),
            bevel: self.bevel.map(BevelEmboss::sanitized),
            satin: self.satin.map(Satin::sanitized),
            gradient_overlay: self.gradient_overlay.map(|overlay| GradientOverlay {
                gradient: overlay.gradient.canonicalized(),
                ..overlay
            }),
        }
    }

    pub(crate) fn scaled(&self, scale: f32) -> Self {
        let glow = |glow: Glow| Glow {
            size: glow.size * scale,
            ..glow
        };
        Self {
            drop_shadow: self.drop_shadow.map(|shadow| shadow.scaled(scale)),
            outer_glow: self.outer_glow.map(glow),
            color_overlay: self.color_overlay,
            inner_glow: self.inner_glow.map(glow),
            inner_shadow: self.inner_shadow.map(|shadow| shadow.scaled(scale)),
            stroke: self.stroke.map(|stroke| LayerStroke {
                size: stroke.size * scale,
                ..stroke
            }),
            bevel: self.bevel.map(|bevel| BevelEmboss {
                size: bevel.size * scale,
                ..bevel
            }),
            satin: self.satin.map(|satin| Satin {
                size: satin.size * scale,
                distance: satin.distance * scale,
                ..satin
            }),
            gradient_overlay: self.gradient_overlay.clone(),
        }
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
            Self::Gradient(gradient) => Self::Gradient(gradient.canonicalized()),
        }
    }

    pub(crate) fn sampler(&self, width: u32, height: u32) -> ShapeFillSampler<'_> {
        match self {
            Self::Gradient(gradient) => ShapeFillSampler {
                gradient: gradient.sampler(),
                width: width.max(1) as f32,
                height: height.max(1) as f32,
            },
        }
    }

    pub(crate) fn uniform_color(&self) -> Option<[u8; 4]> {
        match self {
            Self::Gradient(gradient) => gradient.uniform_color(),
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) struct ShapeFillSampler<'a> {
    gradient: GradientSampler<'a>,
    width: f32,
    height: f32,
}

impl ShapeFillSampler<'_> {
    pub(crate) fn sample(&self, x: f32, y: f32) -> [u8; 4] {
        self.gradient.sample_in_box(
            x.clamp(0.0, self.width),
            y.clamp(0.0, self.height),
            self.width,
            self.height,
        )
    }

    pub(crate) fn sample_alpha(&self, x: f32, y: f32) -> u8 {
        self.gradient.sample_alpha_in_box(
            x.clamp(0.0, self.width),
            y.clamp(0.0, self.height),
            self.width,
            self.height,
        )
    }
}

pub(crate) fn validate_layer_style(style: &LayerStyle) -> Result<()> {
    crate::layer_effects::validate_effects(style)?;
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

/// Half-widths of three box blurs whose result approximates a Gaussian with
/// sigma = radius / 3, so the shadow fades out at about `radius`.
pub(crate) fn shadow_box_halves(radius: f32) -> [usize; 3] {
    if radius < 0.5 {
        return [0; 3];
    }
    let sigma = f64::from(radius) / 3.;
    let ideal = (12. * sigma * sigma / 3. + 1.).sqrt();
    let mut lower = ideal.floor() as i64;
    if lower % 2 == 0 {
        lower -= 1;
    }
    let lower = lower.max(1);
    let upper = lower + 2;
    let lower_f = lower as f64;
    let count = ((12. * sigma * sigma - 3. * lower_f * lower_f - 12. * lower_f - 9.)
        / (-4. * lower_f - 4.))
        .round()
        .clamp(0., 3.) as usize;
    std::array::from_fn(|pass| {
        let width = if pass < count { lower } else { upper };
        ((width - 1) / 2) as usize
    })
}

/// How far a blurred shadow reaches past its source, in pixels.
pub(crate) fn shadow_reach(radius: f32) -> i64 {
    shadow_box_halves(radius).iter().sum::<usize>() as i64
}

/// Blurs an alpha tile in place with three box passes each way, treating
/// pixels outside the tile as clear. The cost does not grow with the radius;
/// rows run in parallel, and columns as rows of the transposed tile.
pub(crate) fn blur_shadow_alpha(pixels: &mut [u8], width: usize, height: usize, radius: f32) {
    use rayon::prelude::*;
    let halves = shadow_box_halves(radius);
    if halves == [0; 3] || width == 0 || height == 0 {
        return;
    }
    let mut values: Vec<f32> = pixels.par_iter().map(|&a| f32::from(a)).collect();
    let mut scratch = vec![0f32; values.len()];
    blur_rows(&mut values, &mut scratch, width, halves);
    let mut prefix = vec![0f32; (height + 1) * width];
    for half in halves {
        box_columns(&values, &mut scratch, &mut prefix, width, half);
        std::mem::swap(&mut values, &mut scratch);
    }
    pixels
        .par_iter_mut()
        .zip(values.par_iter())
        .for_each(|(pixel, value)| *pixel = crate::render::round_byte(*value));
}

/// Runs each box pass along every row of `values`, in parallel.
fn blur_rows(values: &mut Vec<f32>, scratch: &mut Vec<f32>, width: usize, halves: [usize; 3]) {
    use rayon::prelude::*;
    for half in halves {
        scratch
            .par_chunks_mut(width)
            .zip(values.par_chunks(width))
            .for_each(|(output, input)| box_line(input, output, half));
        std::mem::swap(values, scratch);
    }
}

/// One box average down every column: running sums down the rows, then
/// each output row from two of them, in parallel.
fn box_columns(input: &[f32], output: &mut [f32], prefix: &mut [f32], width: usize, half: usize) {
    use rayon::prelude::*;
    let height = input.len() / width;
    prefix[..width].fill(0.0);
    for row in 0..height {
        let (done, rest) = prefix.split_at_mut((row + 1) * width);
        let above = &done[row * width..];
        let line = &input[row * width..(row + 1) * width];
        for ((sum, previous), value) in rest[..width].iter_mut().zip(above).zip(line) {
            *sum = previous + value;
        }
    }
    let span = (2 * half + 1) as f32;
    let prefix = &*prefix;
    output
        .par_chunks_mut(width)
        .enumerate()
        .for_each(|(row, out)| {
            let low = &prefix[row.saturating_sub(half) * width..][..width];
            let high = &prefix[(row + half + 1).min(height) * width..][..width];
            for ((value, high), low) in out.iter_mut().zip(high).zip(low) {
                *value = (high - low) / span;
            }
        });
}

/// A `height`-row tile as `width` rows of `height`, in parallel, a block at
/// a time so reads and writes both stay in cache.
pub(crate) fn transpose<T: Copy + Default + Send + Sync>(
    values: &[T],
    width: usize,
    height: usize,
) -> Vec<T> {
    use rayon::prelude::*;
    const BLOCK: usize = 64;
    let mut out = vec![T::default(); values.len()];
    if height == 0 {
        return out;
    }
    out.par_chunks_mut(height * BLOCK)
        .enumerate()
        .for_each(|(block, columns)| {
            let x0 = block * BLOCK;
            let count = columns.len() / height;
            for y0 in (0..height).step_by(BLOCK) {
                for y in y0..(y0 + BLOCK).min(height) {
                    let row = &values[y * width + x0..y * width + x0 + count];
                    for (dx, value) in row.iter().enumerate() {
                        columns[dx * height + y] = *value;
                    }
                }
            }
        });
    out
}

/// One sliding-window box average along a line.
fn box_line(input: &[f32], output: &mut [f32], half: usize) {
    let length = input.len();
    let span = (2 * half + 1) as f32;
    let mut sum = 0f32;
    for value in &input[..=half.min(length - 1)] {
        sum += value;
    }
    for i in 0..length {
        output[i] = sum / span;
        if i + half + 1 < length {
            sum += input[i + half + 1];
        }
        if i >= half {
            sum -= input[i - half];
        }
    }
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

#[cfg(test)]
mod shadow_blur_tests {
    use super::{blur_shadow_alpha, shadow_reach};

    #[test]
    fn blur_is_smooth_symmetric_and_stays_within_its_reach() {
        let radius = 24.;
        let reach = shadow_reach(radius) as usize;
        let size = 2 * reach + 41;
        let mut tile = vec![0u8; size * size];
        // An opaque 21 x 21 square in the middle.
        for y in reach + 10..reach + 31 {
            for x in reach + 10..reach + 31 {
                tile[y * size + x] = 255;
            }
        }
        let before: u32 = tile.iter().map(|&a| u32::from(a)).sum();
        blur_shadow_alpha(&mut tile, size, size, radius);
        let after: u32 = tile.iter().map(|&a| u32::from(a)).sum();
        // Rounding aside, the blur keeps the amount of shadow.
        assert!(before.abs_diff(after) < before / 50, "{before} vs {after}");
        let row: Vec<u8> = (0..size).map(|x| tile[(size / 2) * size + x]).collect();
        // Mirror symmetric and falling smoothly from the middle outward.
        for x in 0..size {
            assert!(row[x].abs_diff(row[size - 1 - x]) <= 1);
        }
        for x in 1..=size / 2 {
            assert!(row[x] >= row[x - 1], "not monotonic at {x}: {row:?}");
            assert!(row[x] - row[x - 1] < 24, "step too sharp at {x}: {row:?}");
        }
        // Nothing reaches the tile's outer edge.
        assert_eq!(row[0], 0);
    }
}
