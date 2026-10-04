//! Hiding a selection on, or erasing a stroke from, a layer of any kind.
//! Image layers hide selections in their source pixels; everything else —
//! text, shapes, paths, and Paint — keeps a painted alpha on its vector
//! mask, stretched over the layer's box, so it stays editable and the
//! erased part follows it when it moves, scales, or turns.
use anyhow::{Context, Result, bail};
use image::RgbaImage;
use rayon::prelude::*;

use crate::{
    BrushSample, BrushStroke, CommandOutput, Document, LayerKind, PathAnchor, PathFillRule,
    PathGeometry, PixelMask, VectorMask, commands::output,
};

/// The most pixels a painted alpha holds.
const MAX_PAINTED_PIXELS: u64 = 4_096 * 1_024;
/// Painted alpha pixels per canvas pixel along each edge, so erased edges
/// stay sharp on high-density screens.
const PAINTED_DENSITY: f32 = 2.0;

impl VectorMask {
    /// Whether this mask only holds hidden and erased parts, with no path
    /// of its own.
    pub fn painted_only(&self) -> bool {
        whole_box().is_ok_and(|path| path.identity() == self.path.identity())
    }

    /// This mask with every hidden and erased part brought back, or `None`
    /// if nothing else would be left of it.
    pub fn without_painted(&self) -> Option<VectorMask> {
        (!self.painted_only()).then(|| VectorMask {
            alpha: None,
            ..self.clone()
        })
    }
}

/// Whether a layer carries a painted mask: newer encodings only.
pub(crate) fn has_painted_mask(layer: &crate::Layer) -> bool {
    layer
        .vector_mask
        .as_ref()
        .is_some_and(|mask| mask.alpha.is_some())
}

pub(crate) fn validate_painted_alpha(alpha: &PixelMask) -> Result<()> {
    let pixels = u64::from(alpha.width) * u64::from(alpha.height);
    if alpha.width == 0 || alpha.height == 0 || pixels > MAX_PAINTED_PIXELS {
        bail!("a painted mask must be between 1 and {MAX_PAINTED_PIXELS} pixels");
    }
    if alpha.alpha.len() as u64 != pixels {
        bail!("a painted mask's alpha does not match its size");
    }
    if !alpha.has_valid_identity() {
        bail!("a painted mask's identity does not match its alpha");
    }
    Ok(())
}

/// Multiplies `painted`, stretched over a `full`-sized source, into the
/// part of it `image` holds (starting at `origin`), sampled bilinearly.
pub(crate) fn apply_painted_alpha(
    image: &mut RgbaImage,
    painted: &PixelMask,
    full: (u32, u32),
    origin: (u32, u32),
) {
    let (pw, ph) = (painted.width as usize, painted.height as usize);
    let step_x = painted.width as f32 / full.0.max(1) as f32;
    let step_y = painted.height as f32 / full.1.max(1) as f32;
    let at = |x: usize, y: usize| f32::from(painted.alpha[y * pw + x]);
    let width = image.width() as usize;
    image
        .par_chunks_mut(width * 4)
        .enumerate()
        .for_each(|(row, line)| {
            let fy = ((origin.1 as usize + row) as f32 + 0.5) * step_y - 0.5;
            let y0 = fy.floor().clamp(0.0, (ph - 1) as f32) as usize;
            let y1 = (y0 + 1).min(ph - 1);
            let ty = (fy - y0 as f32).clamp(0.0, 1.0);
            for (column, pixel) in line.chunks_exact_mut(4).enumerate() {
                let fx = ((origin.0 as usize + column) as f32 + 0.5) * step_x - 0.5;
                let x0 = fx.floor().clamp(0.0, (pw - 1) as f32) as usize;
                let x1 = (x0 + 1).min(pw - 1);
                let tx = (fx - x0 as f32).clamp(0.0, 1.0);
                let top = at(x0, y0) + (at(x1, y0) - at(x0, y0)) * tx;
                let bottom = at(x0, y1) + (at(x1, y1) - at(x0, y1)) * tx;
                let kept = (top + (bottom - top) * ty).round() as u32;
                pixel[3] = ((u32::from(pixel[3]) * kept + 127) / 255) as u8;
            }
        });
}

/// Where a layer's painted alpha lies on the canvas: its pixels stretch
/// over the layer's box, from the top-left corner along both edges.
struct MaskSpace {
    width: u32,
    height: u32,
    origin: [f32; 2],
    across: [f32; 2],
    down: [f32; 2],
}

impl MaskSpace {
    fn new(document: &Document, id: u64) -> Result<Self> {
        let layer = document.layer(id)?;
        let geometry = crate::document_layer_geometry(document, layer)?;
        let [origin, right, _, bottom] = geometry.corners;
        let across = [right[0] - origin[0], right[1] - origin[1]];
        let down = [bottom[0] - origin[0], bottom[1] - origin[1]];
        let (long, tall) = (across[0].hypot(across[1]), down[0].hypot(down[1]));
        if long < 1.0 || tall < 1.0 {
            bail!("layer {id} is too small to erase from");
        }
        let existing = layer
            .vector_mask
            .as_ref()
            .and_then(|mask| mask.alpha.as_ref());
        let (width, height) = match existing {
            Some(alpha) => (alpha.width, alpha.height),
            None => {
                let (mut width, mut height) = (long * PAINTED_DENSITY, tall * PAINTED_DENSITY);
                let fit = (MAX_PAINTED_PIXELS as f32 / (width * height)).sqrt();
                if fit < 1.0 {
                    (width, height) = (width * fit, height * fit);
                }
                (
                    width.floor().max(1.0) as u32,
                    height.floor().max(1.0) as u32,
                )
            }
        };
        Ok(Self {
            width,
            height,
            origin,
            across,
            down,
        })
    }

    /// The canvas point at the center of a mask pixel.
    fn to_canvas(&self, x: u32, y: u32) -> [f32; 2] {
        let u = (x as f32 + 0.5) / self.width as f32;
        let v = (y as f32 + 0.5) / self.height as f32;
        [
            self.origin[0] + self.across[0] * u + self.down[0] * v,
            self.origin[1] + self.across[1] * u + self.down[1] * v,
        ]
    }

    /// The mask position of a canvas point.
    fn to_mask(&self, [x, y]: [f32; 2]) -> [f32; 2] {
        let (a, d) = (self.across, self.down);
        let det = a[0] * d[1] - a[1] * d[0];
        let (dx, dy) = (x - self.origin[0], y - self.origin[1]);
        let u = (dx * d[1] - dy * d[0]) / det;
        let v = (a[0] * dy - a[1] * dx) / det;
        [u * self.width as f32, v * self.height as f32]
    }

    /// Mask pixels per canvas pixel, averaged over both edges.
    fn density(&self) -> f32 {
        let long = self.across[0].hypot(self.across[1]);
        let tall = self.down[0].hypot(self.down[1]);
        (self.width as f32 / long + self.height as f32 / tall) / 2.0
    }

    /// The mask pixels a canvas-space box (given by points) can touch, as
    /// `x`, `y`, `width`, `height`, or `None` if it misses the layer.
    fn region(&self, points: &[[f32; 2]], reach: f32) -> Option<(u32, u32, u32, u32)> {
        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for point in points {
            let [x, y] = self.to_mask(*point);
            (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
        }
        let left = (x0 - reach).floor().max(0.0) as u32;
        let top = (y0 - reach).floor().max(0.0) as u32;
        let right = ((x1 + reach).ceil().max(0.0) as u32).min(self.width);
        let bottom = ((y1 + reach).ceil().max(0.0) as u32).min(self.height);
        (right > left && bottom > top).then(|| (left, top, right - left, bottom - top))
    }
}

/// Hides what the selection covers on layer `id`.
pub(crate) fn hide_selection(document: &mut Document, id: u64) -> Result<CommandOutput> {
    let layer = document.layer(id)?;
    if layer.locked {
        bail!("layer {id} is locked");
    }
    if matches!(layer.kind, LayerKind::Raster { .. }) {
        crate::pixel_masks::delete_selected_raster_pixels(document, id)?;
        return Ok(output(
            "hide_selection",
            "hid the selected pixels",
            vec![id],
        ));
    }
    let selection = document
        .selection
        .clone()
        .context("select something to hide first")?;
    if !shows_under(document, id, &selection)? {
        bail!("the selection does not cover any visible part of layer {id}");
    }
    let space = MaskSpace::new(document, id)?;
    let (sx, sy, sw, sh) = selection.bounds();
    let (sx, sy, sw, sh) = (sx as f32, sy as f32, sw as f32, sh as f32);
    let corners = [[sx, sy], [sx + sw, sy], [sx, sy + sh], [sx + sw, sy + sh]];
    let region = space
        .region(&corners, 1.0)
        .with_context(|| format!("the selection does not cover layer {id}"))?;
    let mut coverage = vec![0_u8; (region.2 * region.3) as usize];
    coverage
        .par_chunks_mut(region.2 as usize)
        .enumerate()
        .for_each(|(row, line)| {
            for (column, value) in line.iter_mut().enumerate() {
                let [x, y] = space.to_canvas(region.0 + column as u32, region.1 + row as u32);
                if x >= 0.0 && y >= 0.0 {
                    *value = selection.alpha_at(x as u32, y as u32);
                }
            }
        });
    erase(document, id, &space, region, &coverage)?;
    Ok(output("hide_selection", "hid the selected part", vec![id]))
}

/// Whether any of layer `id`'s own pixels (without its styles) show under
/// the selection, so hiding there changes something.
fn shows_under(document: &Document, id: u64, selection: &crate::Selection) -> Result<bool> {
    let mut alone = document.clone();
    alone.layers.retain(|layer| layer.id == id);
    alone.background = [0; 4];
    alone.selection = None;
    for layer in &mut alone.layers {
        layer.visible = true;
        layer.opacity = 1.0;
        layer.clip_to_below = false;
        layer.blend_mode = crate::BlendMode::Normal;
        layer.style = crate::LayerStyle::default();
    }
    let (x, y, width, height) = selection.bounds();
    let width = width.min(document.width.saturating_sub(x));
    let height = height.min(document.height.saturating_sub(y));
    if width == 0 || height == 0 {
        return Ok(false);
    }
    let region = crate::RenderRegion {
        x,
        y,
        width,
        height,
    };
    let pixels = crate::render_document_region_scaled(&alone, 1.0, region)?.into_rgba8();
    Ok(pixels
        .enumerate_pixels()
        .any(|(px, py, pixel)| pixel[3] > 0 && selection.alpha_at(x + px, y + py) > 0))
}

/// Erases `stroke`, drawn in canvas coordinates, from layer `id`, inside
/// the selection if there is one.
pub(crate) fn erase_stroke(
    document: &mut Document,
    id: u64,
    stroke: &BrushStroke,
) -> Result<CommandOutput> {
    if document.layer(id)?.locked {
        bail!("layer {id} is locked");
    }
    let space = MaskSpace::new(document, id)?;
    let density = space.density();
    let samples: Vec<BrushSample> = stroke
        .samples
        .iter()
        .map(|sample| {
            let [x, y] = space.to_mask([sample.x, sample.y]);
            BrushSample { x, y, ..*sample }
        })
        .collect();
    let mut style = stroke.style;
    style.size = (style.size * density).clamp(1.0, 2_048.0);
    let local = BrushStroke::new(style, samples)?;
    let points: Vec<[f32; 2]> = stroke.samples.iter().map(|s| [s.x, s.y]).collect();
    let region = space
        .region(&points, style.size / 2.0 + 1.0)
        .with_context(|| format!("the stroke does not touch layer {id}"))?;
    let mut coverage = crate::paint_render::stroke_coverage(&local, region);
    // Like the Brush, the Eraser keeps inside the selection.
    if let Some(selection) = &document.selection {
        for (index, value) in coverage.iter_mut().enumerate() {
            let column = index as u32 % region.2;
            let row = index as u32 / region.2;
            let [x, y] = space.to_canvas(region.0 + column, region.1 + row);
            let inside = if x >= 0.0 && y >= 0.0 {
                selection.alpha_at(x as u32, y as u32)
            } else {
                0
            };
            *value = ((u32::from(*value) * u32::from(inside) + 127) / 255) as u8;
        }
    }
    erase(document, id, &space, region, &coverage)?;
    Ok(output("erase_layer", "erased part of the layer", vec![id]))
}

/// Takes `coverage` over `region` out of layer `id`'s painted alpha.
fn erase(
    document: &mut Document,
    id: u64,
    space: &MaskSpace,
    region: (u32, u32, u32, u32),
    coverage: &[u8],
) -> Result<()> {
    if coverage.iter().all(|value| *value == 0) {
        bail!("nothing to erase on layer {id} there");
    }
    let current = document.layer(id)?.vector_mask.clone();
    let existing = current.as_ref().and_then(|mask| mask.alpha.as_ref());
    let mut alpha = existing.map_or_else(
        || vec![255; (space.width * space.height) as usize],
        |alpha| alpha.alpha.to_vec(),
    );
    for row in 0..region.3 {
        for column in 0..region.2 {
            let taken = coverage[(row * region.2 + column) as usize];
            let index = ((region.1 + row) * space.width + region.0 + column) as usize;
            alpha[index] = ((u32::from(alpha[index]) * u32::from(255 - taken) + 127) / 255) as u8;
        }
    }
    document.validate_projected_inline_mask_budget(
        document.selection.as_ref(),
        if existing.is_none() { alpha.len() } else { 0 },
    )?;
    let mut mask = match current {
        Some(mask) => mask,
        None => VectorMask::new(whole_box()?, false)?,
    };
    mask.alpha = Some(PixelMask::new(space.width, space.height, alpha));
    document.layer_mut(id)?.vector_mask = Some(mask);
    document.validate_inline_mask_budget()
}

/// A path covering its whole box: masks with only a painted alpha.
fn whole_box() -> Result<PathGeometry> {
    PathGeometry::new(
        64,
        64,
        true,
        PathFillRule::EvenOdd,
        [[0.0, 0.0], [64.0, 0.0], [64.0, 64.0], [0.0, 64.0]]
            .map(|[x, y]| PathAnchor::corner(x, y))
            .to_vec(),
    )
}
