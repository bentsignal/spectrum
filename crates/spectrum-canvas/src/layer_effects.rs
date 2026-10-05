//! Photoshop's classic layer styles beyond the drop shadow: stroke, outer and
//! inner glow, inner shadow, and color overlay. Each is computed from the
//! layer's alpha in a padded tile and drawn behind or above the layer.
use anyhow::{Result, bail};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::{
    DropShadow, LayerStyle,
    effects::{blur_shadow_alpha, shadow_reach, transpose},
    validation::require_finite,
};

/// The largest stroke, glow, or inner-shadow size, in canvas pixels.
pub const MAX_EFFECT_SIZE: f32 = 250.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StrokePosition {
    #[default]
    Outside,
    Inside,
    Center,
}

/// An outline following the layer's edge, as Photoshop's Stroke style.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LayerStroke {
    pub size: f32,
    pub position: StrokePosition,
    pub color: [u8; 4],
}

impl Default for LayerStroke {
    fn default() -> Self {
        Self {
            size: 3.0,
            position: StrokePosition::Outside,
            color: [0, 0, 0, 255],
        }
    }
}

/// A soft light around the outside or inside of the layer's edge. `spread`
/// (0 to 1) is the part of `size` that grows solidly before fading.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Glow {
    pub color: [u8; 4],
    pub size: f32,
    pub spread: f32,
}

impl Default for Glow {
    fn default() -> Self {
        Self {
            color: [255, 255, 190, 190],
            size: 16.0,
            spread: 0.0,
        }
    }
}

/// The layer's shape filled with one color; its alpha is the strength.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ColorOverlay {
    pub color: [u8; 4],
}

impl Default for ColorOverlay {
    fn default() -> Self {
        Self {
            color: [255, 64, 64, 255],
        }
    }
}

impl LayerStroke {
    pub(crate) fn sanitized(self) -> Self {
        Self {
            size: self.size.clamp(0.0, MAX_EFFECT_SIZE),
            ..self
        }
    }
}

impl Glow {
    pub(crate) fn sanitized(self) -> Self {
        Self {
            size: self.size.clamp(0.0, MAX_EFFECT_SIZE),
            spread: self.spread.clamp(0.0, 1.0),
            ..self
        }
    }
}

pub(crate) fn validate_effects(style: &LayerStyle) -> Result<()> {
    if let Some(shadow) = style.inner_shadow {
        require_finite("inner shadow horizontal offset", shadow.offset_x)?;
        require_finite("inner shadow vertical offset", shadow.offset_y)?;
        require_finite("inner shadow blur radius", shadow.blur_radius)?;
        if shadow.blur_radius < 0.0 {
            bail!("inner shadow blur radius cannot be negative");
        }
    }
    for (name, glow) in [
        ("outer glow", style.outer_glow),
        ("inner glow", style.inner_glow),
    ] {
        if let Some(glow) = glow {
            require_finite(&format!("{name} size"), glow.size)?;
            require_finite(&format!("{name} spread"), glow.spread)?;
        }
    }
    if let Some(stroke) = style.stroke {
        require_finite("stroke size", stroke.size)?;
    }
    if let Some(bevel) = style.bevel {
        for (name, value) in [
            ("bevel size", bevel.size),
            ("bevel depth", bevel.depth),
            ("bevel angle", bevel.angle),
            ("bevel altitude", bevel.altitude),
        ] {
            require_finite(name, value)?;
        }
    }
    if let Some(satin) = style.satin {
        for (name, value) in [
            ("satin angle", satin.angle),
            ("satin distance", satin.distance),
            ("satin size", satin.size),
        ] {
            require_finite(name, value)?;
        }
    }
    if let Some(overlay) = &style.gradient_overlay {
        overlay.gradient.validate().map_err(anyhow::Error::new)?;
    }
    Ok(())
}

/// How far, in render pixels, the styles here reach past the layer's edge,
/// and how much context around it they need.
pub(crate) fn effects_reach(style: &LayerStyle) -> i64 {
    let glow = |glow: Option<Glow>| {
        glow.map_or(0, |g| {
            (g.size * g.spread).ceil() as i64 + shadow_reach(g.size * (1.0 - g.spread)) + 1
        })
    };
    let stroke = style.stroke.map_or(0, |s| s.size.ceil() as i64 + 2);
    let inner = style.inner_shadow.map_or(0, |s| {
        s.offset_x.abs().max(s.offset_y.abs()).ceil() as i64 + shadow_reach(s.blur_radius) + 1
    });
    glow(style.outer_glow)
        .max(glow(style.inner_glow))
        .max(stroke)
        .max(inner)
        .max(style.bevel.map_or(0, |b| b.reach()))
        .max(style.satin.map_or(0, |s| s.reach()))
        .max(i64::from(
            style.color_overlay.is_some() || style.gradient_overlay.is_some(),
        ))
}

/// How far a layer's styles, including its drop shadow, can draw past the
/// layer's edge, in canvas pixels.
pub fn style_reach(style: &LayerStyle) -> f32 {
    let shadow = style.drop_shadow.map_or(0.0, |s| {
        s.offset_x.abs().max(s.offset_y.abs()) + shadow_reach(s.blur_radius) as f32 + 2.0
    });
    shadow.max(effects_reach(style) as f32 + 1.0)
}

pub(crate) fn has_effects(style: &LayerStyle) -> bool {
    style.stroke.is_some()
        || style.outer_glow.is_some()
        || style.inner_glow.is_some()
        || style.inner_shadow.is_some()
        || style.color_overlay.is_some()
        || style.gradient_overlay.is_some()
        || style.bevel.is_some()
        || style.satin.is_some()
}

/// The layer's alpha over an area, in the layer's output pixels.
pub(crate) struct AlphaTile {
    pub left: i64,
    pub top: i64,
    pub width: usize,
    pub height: usize,
    pub alpha: Vec<u8>,
    /// The layer's own size in the same pixels, for gradients across it.
    pub extent: [f32; 2],
}

/// One style to draw: a color and its strength over the tile's area.
pub(crate) struct EffectPass {
    pub color: [u8; 4],
    pub alpha: Vec<u8>,
    pub mode: crate::BlendMode,
    /// A color per pixel, as for a gradient; its alpha scales `alpha`.
    pub colors: Option<Vec<[u8; 4]>>,
}

impl EffectPass {
    fn solid(color: [u8; 4], alpha: Vec<u8>) -> Self {
        Self {
            color,
            alpha,
            mode: crate::BlendMode::Normal,
            colors: None,
        }
    }
}

/// The styles drawn behind the layer and above it, in drawing order.
pub(crate) fn effect_passes(
    style: &LayerStyle,
    tile: &AlphaTile,
) -> (Vec<EffectPass>, Vec<EffectPass>) {
    let (width, height, alpha) = (tile.width, tile.height, &tile.alpha);
    let mut behind = Vec::new();
    let mut above = Vec::new();
    // Distances are the costly part; each is computed at most once.
    let outside_cell = std::cell::OnceCell::new();
    let inside_cell = std::cell::OnceCell::new();
    let outside = || {
        outside_cell
            .get_or_init(|| distance_to(alpha, width, height, |a| a >= 128))
            .as_slice()
    };
    let inside = || {
        inside_cell
            .get_or_init(|| distance_to(alpha, width, height, |a| a < 128))
            .as_slice()
    };
    if let Some(glow) = style.outer_glow {
        let grown = grow(alpha, || outside(), glow.size * glow.spread);
        behind.push(EffectPass::solid(
            glow.color,
            blurred(grown, width, height, glow.size * (1.0 - glow.spread)),
        ));
    }
    if let Some(overlay) = &style.gradient_overlay {
        above.push(crate::layer_shading::gradient_overlay_pass(overlay, tile));
    }
    if let Some(overlay) = style.color_overlay {
        above.push(EffectPass::solid(overlay.color, alpha.clone()));
    }
    if let Some(satin) = &style.satin {
        above.push(crate::layer_shading::satin_pass(satin, tile));
    }
    if let Some(glow) = style.inner_glow {
        let inverse: Vec<u8> = alpha.par_iter().map(|a| 255 - a).collect();
        let grown = grow(&inverse, || inside(), glow.size * glow.spread);
        let edge = blurred(grown, width, height, glow.size * (1.0 - glow.spread));
        above.push(EffectPass::solid(glow.color, within(&edge, alpha)));
    }
    if let Some(shadow) = style.inner_shadow {
        above.push(EffectPass::solid(
            shadow.color,
            inner_shadow(alpha, width, height, shadow),
        ));
    }
    if let Some(bevel) = &style.bevel {
        above.extend(crate::layer_shading::bevel_passes(bevel, tile));
    }
    if let Some(stroke) = style.stroke.filter(|s| s.size > 0.0) {
        let (out, inn) = match stroke.position {
            StrokePosition::Outside => (stroke.size, 0.0),
            StrokePosition::Inside => (0.0, stroke.size),
            StrokePosition::Center => (stroke.size / 2.0, stroke.size / 2.0),
        };
        let mut band = vec![0u8; alpha.len()];
        if out > 0.0 {
            let grown = grow(alpha, || outside(), out);
            for ((b, g), a) in band.iter_mut().zip(grown).zip(alpha) {
                *b = (u16::from(g) * u16::from(255 - a) / 255) as u8;
            }
        }
        if inn > 0.0 {
            let to_edge = inside();
            for ((b, d), a) in band.iter_mut().zip(to_edge).zip(alpha) {
                let near = (inn + 1.0 - d).clamp(0.0, 1.0);
                *b = b.saturating_add(crate::render::round_byte(f32::from(*a) * near));
            }
        }
        above.push(EffectPass::solid(stroke.color, band));
    }
    (behind, above)
}

/// Alpha grown outward by `radius` pixels, given each pixel's distance to
/// the shape; edges stay antialiased.
fn grow<'a>(alpha: &[u8], distance: impl FnOnce() -> &'a [f32], radius: f32) -> Vec<u8> {
    if radius <= 0.0 {
        return alpha.to_vec();
    }
    let distance = distance();
    alpha
        .par_iter()
        .zip(distance)
        .map(|(&a, &d)| {
            if d == 0.0 {
                a
            } else {
                a.max(crate::render::round_byte(
                    (radius + 1.0 - d).clamp(0.0, 1.0) * 255.0,
                ))
            }
        })
        .collect()
}

fn blurred(mut alpha: Vec<u8>, width: usize, height: usize, radius: f32) -> Vec<u8> {
    blur_shadow_alpha(&mut alpha, width, height, radius);
    alpha
}

fn within(effect: &[u8], alpha: &[u8]) -> Vec<u8> {
    effect
        .par_iter()
        .zip(alpha)
        .map(|(&e, &a)| (u16::from(e) * u16::from(a) / 255) as u8)
        .collect()
}

/// The shadow the layer's surroundings cast inward, offset and blurred.
fn inner_shadow(alpha: &[u8], width: usize, height: usize, shadow: DropShadow) -> Vec<u8> {
    let (dx, dy) = (
        shadow.offset_x.round() as i64,
        shadow.offset_y.round() as i64,
    );
    let mut cast = vec![0u8; alpha.len()];
    cast.par_chunks_mut(width.max(1))
        .enumerate()
        .for_each(|(y, row)| {
            for (x, value) in row.iter_mut().enumerate() {
                let (sx, sy) = (x as i64 - dx, y as i64 - dy);
                let covered = if sx < 0 || sy < 0 || sx >= width as i64 || sy >= height as i64 {
                    0
                } else {
                    alpha[sy as usize * width + sx as usize]
                };
                *value = 255 - covered;
            }
        });
    within(&blurred(cast, width, height, shadow.blur_radius), alpha)
}

/// Euclidean distance from each pixel to the nearest pixel where `target`
/// holds (0 there), by Felzenszwalb and Huttenlocher's two-pass transform.
/// Columns run as rows of the transposed tile; rows run in parallel.
fn distance_to(
    alpha: &[u8],
    width: usize,
    height: usize,
    target: impl Fn(u8) -> bool + Sync,
) -> Vec<f32> {
    const FAR: f32 = 1e12;
    let squared: Vec<f32> = alpha
        .par_iter()
        .map(|&a| if target(a) { 0.0 } else { FAR })
        .collect();
    let mut columns = transpose(&squared, width, height);
    columns
        .par_chunks_mut(height.max(1))
        .for_each_init(LineScratch::default, |scratch, line| {
            scratch.transform(line)
        });
    let mut rows = transpose(&columns, height, width);
    rows.par_chunks_mut(width.max(1))
        .for_each_init(LineScratch::default, |scratch, line| {
            scratch.transform(line);
            for value in line {
                *value = value.sqrt();
            }
        });
    rows
}

#[derive(Default)]
struct LineScratch {
    f: Vec<f32>,
    hull: Vec<usize>,
    bounds: Vec<f32>,
}

impl LineScratch {
    /// Replaces squared distances along one line with the lower envelope of
    /// parabolas rooted at each sample.
    fn transform(&mut self, line: &mut [f32]) {
        let n = line.len();
        if n == 0 {
            return;
        }
        self.f.clear();
        self.f.extend_from_slice(line);
        self.hull.clear();
        self.hull.resize(n, 0);
        self.bounds.clear();
        self.bounds.resize(n + 1, 0.0);
        let (f, hull, bounds) = (&self.f, &mut self.hull, &mut self.bounds);
        let intersect = |q: usize, p: usize| {
            let (qf, pf) = (q as f32, p as f32);
            ((f[q] + qf * qf) - (f[p] + pf * pf)) / (2.0 * qf - 2.0 * pf)
        };
        let mut k = 0;
        bounds[0] = f32::NEG_INFINITY;
        bounds[1] = f32::INFINITY;
        for q in 1..n {
            let mut s = intersect(q, hull[k]);
            while s <= bounds[k] {
                k -= 1;
                s = intersect(q, hull[k]);
            }
            k += 1;
            hull[k] = q;
            bounds[k] = s;
            bounds[k + 1] = f32::INFINITY;
        }
        k = 0;
        for (q, value) in line.iter_mut().enumerate() {
            while bounds[k + 1] < q as f32 {
                k += 1;
            }
            let d = q as f32 - hull[k] as f32;
            *value = d * d + f[hull[k]];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(size: usize, inner: std::ops::Range<usize>) -> AlphaTile {
        let mut alpha = vec![0u8; size * size];
        for y in inner.clone() {
            for x in inner.clone() {
                alpha[y * size + x] = 255;
            }
        }
        AlphaTile {
            left: 0,
            top: 0,
            width: size,
            height: size,
            alpha,
            extent: [size as f32; 2],
        }
    }

    #[test]
    fn distances_are_euclidean() {
        let tile = square(9, 4..5);
        let d = distance_to(&tile.alpha, 9, 9, |a| a >= 128);
        assert_eq!(d[4 * 9 + 4], 0.0);
        assert_eq!(d[4 * 9 + 7], 3.0);
        assert!((d[7 * 9 + 7] - 18f32.sqrt()).abs() < 1e-4);
    }

    #[test]
    fn outside_stroke_rings_the_shape_only() {
        let tile = square(20, 6..14);
        let style = LayerStyle {
            stroke: Some(LayerStroke {
                size: 2.0,
                ..LayerStroke::default()
            }),
            ..LayerStyle::default()
        };
        let (behind, above) = effect_passes(&style, &tile);
        assert!(behind.is_empty());
        let band = &above[0].alpha;
        assert_eq!(band[10 * 20 + 10], 0, "inside stays clear");
        assert_eq!(band[10 * 20 + 5], 255, "one pixel out");
        assert_eq!(band[10 * 20 + 4], 255, "two pixels out");
        assert_eq!(band[10 * 20 + 2], 0, "past the stroke");
    }

    #[test]
    fn inside_stroke_and_overlay_stay_within_the_shape() {
        let tile = square(20, 6..14);
        let style = LayerStyle {
            stroke: Some(LayerStroke {
                size: 2.0,
                position: StrokePosition::Inside,
                ..LayerStroke::default()
            }),
            color_overlay: Some(ColorOverlay::default()),
            ..LayerStyle::default()
        };
        let (_, above) = effect_passes(&style, &tile);
        let (overlay, band) = (&above[0].alpha, &above[1].alpha);
        assert_eq!(overlay, &tile.alpha);
        assert_eq!(band[10 * 20 + 6], 255);
        assert_eq!(band[10 * 20 + 7], 255);
        assert_eq!(band[10 * 20 + 10], 0);
        assert_eq!(band[10 * 20 + 5], 0);
    }

    #[test]
    fn glows_fade_away_from_the_edge() {
        let tile = square(60, 20..40);
        let style = LayerStyle {
            outer_glow: Some(Glow::default()),
            inner_glow: Some(Glow::default()),
            inner_shadow: Some(DropShadow::default()),
            ..LayerStyle::default()
        };
        let (behind, above) = effect_passes(&style, &tile);
        let outer = &behind[0].alpha;
        assert!(outer[30 * 60 + 18] > outer[30 * 60 + 12]);
        assert!(outer[30 * 60 + 12] > outer[30 * 60 + 2]);
        let inner = &above[0].alpha;
        assert!(inner[30 * 60 + 21] > inner[30 * 60 + 28]);
        assert_eq!(inner[30 * 60 + 10], 0, "inner glow stays inside");
        let shadow = &above[1].alpha;
        assert!(
            shadow[21 * 60 + 21] > shadow[38 * 60 + 38],
            "cast from the top left"
        );
        assert!(effects_reach(&style) >= 16);
    }
}
