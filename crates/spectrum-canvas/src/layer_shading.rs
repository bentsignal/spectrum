//! The shading layer styles: bevel and emboss, satin, and gradient overlay.
//! Each turns the layer's alpha into one or more passes for
//! `layer_effects::effect_passes` to draw in Photoshop's order.
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::{
    BlendMode, ShapeGradient,
    effects::{blur_shadow_alpha, shadow_reach},
    layer_effects::{AlphaTile, EffectPass, MAX_EFFECT_SIZE},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BevelStyle {
    /// Raised inside the edge.
    #[default]
    Inner,
    /// Raised outside the edge, as if the canvas rose to meet the layer.
    Outer,
    /// Raised across the edge, inside and out.
    Emboss,
    /// The edge pressed into the canvas.
    Pillow,
}

/// Light and shade across a raised edge, as Photoshop's Bevel & Emboss.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BevelEmboss {
    pub style: BevelStyle,
    /// How far the slope reaches from the edge, in canvas pixels.
    pub size: f32,
    /// Steepness, 1 for Photoshop's 100%.
    pub depth: f32,
    /// Where light comes from in degrees: 0 from the right, 90 from above.
    pub angle: f32,
    /// The light's height above the canvas in degrees.
    pub altitude: f32,
    /// False presses the bevel down instead of raising it.
    pub up: bool,
    pub highlight: [u8; 4],
    pub shadow: [u8; 4],
}

impl Default for BevelEmboss {
    fn default() -> Self {
        Self {
            style: BevelStyle::Inner,
            size: 8.0,
            depth: 1.0,
            angle: 120.0,
            altitude: 30.0,
            up: true,
            highlight: [255, 255, 255, 190],
            shadow: [0, 0, 0, 190],
        }
    }
}

/// Soft interior shading from the layer's shape overlapping itself.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Satin {
    pub color: [u8; 4],
    pub angle: f32,
    pub distance: f32,
    pub size: f32,
    pub invert: bool,
    pub blend_mode: BlendMode,
}

impl Default for Satin {
    fn default() -> Self {
        Self {
            color: [0, 0, 0, 128],
            angle: 19.0,
            distance: 11.0,
            size: 14.0,
            invert: true,
            blend_mode: BlendMode::Multiply,
        }
    }
}

/// The layer's shape filled with a gradient across its bounds.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GradientOverlay {
    pub gradient: ShapeGradient,
    pub blend_mode: BlendMode,
}

impl Default for GradientOverlay {
    fn default() -> Self {
        Self {
            gradient: ShapeGradient::default(),
            blend_mode: BlendMode::Normal,
        }
    }
}

impl BevelEmboss {
    pub(crate) fn sanitized(self) -> Self {
        Self {
            size: self.size.clamp(0.0, MAX_EFFECT_SIZE),
            depth: self.depth.clamp(0.0, 10.0),
            altitude: self.altitude.clamp(0.0, 90.0),
            ..self
        }
    }

    pub(crate) fn reach(&self) -> i64 {
        self.size.ceil() as i64 + shadow_reach(self.size) + 2
    }
}

impl Satin {
    pub(crate) fn sanitized(self) -> Self {
        Self {
            size: self.size.clamp(0.0, MAX_EFFECT_SIZE),
            distance: self.distance.clamp(0.0, MAX_EFFECT_SIZE),
            ..self
        }
    }

    pub(crate) fn reach(&self) -> i64 {
        self.distance.ceil() as i64 + shadow_reach(self.size) + 1
    }
}

/// Unit light direction toward the light, in tile coordinates (y down), and
/// the shading a flat surface gets.
fn light(angle: f32, altitude: f32) -> ([f32; 3], f32) {
    let (theta, phi) = (angle.to_radians(), altitude.to_radians());
    let l = [theta.cos() * phi.cos(), -theta.sin() * phi.cos(), phi.sin()];
    (l, phi.sin())
}

/// Highlight and shadow passes for a bevel.
pub(crate) fn bevel_passes(bevel: &BevelEmboss, tile: &AlphaTile) -> [EffectPass; 2] {
    let (width, height, alpha) = (tile.width, tile.height, &tile.alpha);
    // The surface rises across the edge: the alpha blurred over the bevel's
    // size. Emboss and pillow center the slope on the edge; inner keeps it
    // inside and outer outside, by where the shading is drawn.
    let mut surface = alpha.clone();
    blur_shadow_alpha(&mut surface, width, height, bevel.size);
    let (toward, flat) = light(bevel.angle, bevel.altitude);
    let sign = if bevel.up { 1.0 } else { -1.0 };
    // Height in pixels per unit of surface: the full rise over `size`.
    let rise = bevel.depth * bevel.size.max(1.0) / 255.0;
    let mut highlight = vec![0u8; alpha.len()];
    let mut shadow = vec![0u8; alpha.len()];
    highlight
        .par_chunks_mut(width.max(1))
        .zip(shadow.par_chunks_mut(width.max(1)))
        .enumerate()
        .for_each(|(y, (light_row, dark_row))| {
            let at = |x: usize, y: usize| f32::from(surface[y * width + x]);
            for x in 0..width {
                let a = alpha[y * width + x];
                // Where the shading shows: inside, outside, or both.
                let (inside, outside) = (f32::from(a) / 255.0, 1.0 - f32::from(a) / 255.0);
                let (coverage, flip) = match bevel.style {
                    BevelStyle::Inner => (inside, 1.0),
                    BevelStyle::Outer => (outside, 1.0),
                    BevelStyle::Emboss => (1.0, 1.0),
                    BevelStyle::Pillow => (1.0, if a >= 128 { 1.0 } else { -1.0 }),
                };
                if coverage <= 0.0 {
                    continue;
                }
                let (left, right) = (x.saturating_sub(1), (x + 1).min(width - 1));
                let (up, down) = (y.saturating_sub(1), (y + 1).min(height - 1));
                let dx = (at(right, y) - at(left, y)) / (right - left).max(1) as f32;
                let dy = (at(x, down) - at(x, up)) / (down - up).max(1) as f32;
                let (nx, ny) = (-dx * rise * sign * flip, -dy * rise * sign * flip);
                let length = (nx * nx + ny * ny + 1.0).sqrt();
                let shade = (nx * toward[0] + ny * toward[1] + toward[2]) / length;
                let (lit, dark) = if shade >= flat {
                    ((shade - flat) / (1.0 - flat).max(1e-3), 0.0)
                } else {
                    (0.0, (flat - shade) / flat.max(1e-3))
                };
                light_row[x] = crate::render::round_byte(lit.min(1.0) * coverage * 255.0);
                dark_row[x] = crate::render::round_byte(dark.min(1.0) * coverage * 255.0);
            }
        });
    [
        EffectPass {
            color: bevel.highlight,
            alpha: highlight,
            mode: BlendMode::Screen,
            colors: None,
        },
        EffectPass {
            color: bevel.shadow,
            alpha: shadow,
            mode: BlendMode::Multiply,
            colors: None,
        },
    ]
}

/// Satin: the shape offset both ways along `angle`, blurred, and differenced.
pub(crate) fn satin_pass(satin: &Satin, tile: &AlphaTile) -> EffectPass {
    let (width, height, alpha) = (tile.width, tile.height, &tile.alpha);
    let (dx, dy) = (
        (satin.angle.to_radians().cos() * satin.distance).round() as i64,
        (-satin.angle.to_radians().sin() * satin.distance).round() as i64,
    );
    let shifted = |sx: i64, sy: i64| {
        let mut out = vec![0u8; alpha.len()];
        out.par_chunks_mut(width.max(1))
            .enumerate()
            .for_each(|(y, row)| {
                for (x, value) in row.iter_mut().enumerate() {
                    let (fx, fy) = (x as i64 - sx, y as i64 - sy);
                    if fx >= 0 && fy >= 0 && fx < width as i64 && fy < height as i64 {
                        *value = alpha[fy as usize * width + fx as usize];
                    }
                }
            });
        blur_shadow_alpha(&mut out, width, height, satin.size);
        out
    };
    let (one, other) = (shifted(dx, dy), shifted(-dx, -dy));
    let strength = one
        .par_iter()
        .zip(other.par_iter())
        .zip(alpha.par_iter())
        .map(|((&p, &q), &a)| {
            let difference = p.abs_diff(q);
            let value = if satin.invert {
                255 - difference
            } else {
                difference
            };
            (u16::from(value) * u16::from(a) / 255) as u8
        })
        .collect();
    EffectPass {
        color: satin.color,
        alpha: strength,
        mode: satin.blend_mode,
        colors: None,
    }
}

/// The gradient across the layer's bounds, within its shape.
pub(crate) fn gradient_overlay_pass(overlay: &GradientOverlay, tile: &AlphaTile) -> EffectPass {
    let sampler = overlay.gradient.sampler();
    let (box_width, box_height) = (tile.extent[0].max(1.0), tile.extent[1].max(1.0));
    let mut colors = vec![[0u8; 4]; tile.alpha.len()];
    colors
        .par_chunks_mut(tile.width.max(1))
        .enumerate()
        .for_each(|(row, line)| {
            let y = (tile.top + row as i64) as f32 + 0.5;
            for (column, color) in line.iter_mut().enumerate() {
                let x = (tile.left + column as i64) as f32 + 0.5;
                *color = sampler.sample_in_box(
                    x.clamp(0.0, box_width),
                    y.clamp(0.0, box_height),
                    box_width,
                    box_height,
                );
            }
        });
    EffectPass {
        color: [255, 255, 255, 255],
        alpha: tile.alpha.clone(),
        mode: overlay.blend_mode,
        colors: Some(colors),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn disc() -> AlphaTile {
        let size = 60usize;
        let alpha = (0..size * size)
            .map(|i| {
                let (x, y) = ((i % size) as f32 - 30.0, (i / size) as f32 - 30.0);
                if x * x + y * y < 18.0 * 18.0 { 255 } else { 0 }
            })
            .collect();
        AlphaTile {
            left: -10,
            top: -10,
            width: size,
            height: size,
            alpha,
            extent: [40.0, 40.0],
        }
    }

    #[test]
    fn bevel_lights_the_side_facing_the_light() {
        let tile = disc();
        let [lit, dark] = bevel_passes(&BevelEmboss::default(), &tile);
        // Light from the upper left: the upper-left rim is lit, the lower
        // right is in shade, and the flat middle is neither.
        let (upper_left, lower_right) = (19 * 60 + 19, 41 * 60 + 41);
        assert!(lit.alpha[upper_left] > lit.alpha[lower_right]);
        assert!(dark.alpha[lower_right] > dark.alpha[upper_left]);
        assert_eq!(lit.alpha[30 * 60 + 30], 0);
        assert_eq!(lit.alpha[2 * 60 + 2], 0, "inner bevels stay inside");
    }

    #[test]
    fn satin_and_gradient_stay_within_the_shape() {
        let tile = disc();
        let satin = satin_pass(&Satin::default(), &tile);
        assert_eq!(satin.alpha[0], 0);
        assert!(satin.alpha.iter().any(|&a| a > 0));
        let overlay = gradient_overlay_pass(&GradientOverlay::default(), &tile);
        let colors = overlay.colors.unwrap();
        assert_ne!(
            colors[30 * 60 + 12],
            colors[30 * 60 + 48],
            "varies across the layer"
        );
        assert_eq!(overlay.alpha, tile.alpha);
    }
}
