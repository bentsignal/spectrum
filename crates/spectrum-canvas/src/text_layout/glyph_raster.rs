use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock},
};

use anyhow::{Context, Result, bail};
use tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, Transform};
use ttf_parser::{Face, GlyphId, OutlineBuilder, Rect};

const MAX_GLYPH_PIXELS: u64 = 4_096 * 4_096;
/// Rendered glyphs kept for reuse; the cache empties when it grows past this.
const MAX_CACHED_GLYPH_BYTES: usize = 64 * 1024 * 1024;

pub(super) struct GlyphBitmap {
    pub(super) left: i32,
    pub(super) top: i32,
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) alpha: Arc<[u8]>,
}

/// A glyph's coverage depends only on its font, glyph, and size; where it
/// lands only offsets it. Text re-renders on every slider step, so glyphs
/// are kept by font content, glyph, and size.
type GlyphKey = (Arc<str>, u16, u32);

#[derive(Default)]
struct GlyphCache {
    glyphs: HashMap<GlyphKey, Option<Arc<[u8]>>>,
    bytes: usize,
}

fn glyph_cache() -> &'static Mutex<GlyphCache> {
    static CACHE: OnceLock<Mutex<GlyphCache>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

#[derive(Clone, Copy)]
pub(super) struct GlyphPixelBounds {
    pub(super) left: i32,
    pub(super) top: i32,
    pub(super) width: u32,
    pub(super) height: u32,
}

pub(super) fn glyph_pixel_bounds(
    face: &Face<'_>,
    glyph_id: u16,
    font_size: f32,
    pen_x: f32,
    baseline: f32,
    x_offset_units: i32,
    y_offset_units: i32,
) -> Option<GlyphPixelBounds> {
    let bounds = face.glyph_bounding_box(GlyphId(glyph_id))?;
    let scale = font_size / f32::from(face.units_per_em());
    let width = ((f32::from(bounds.x_max - bounds.x_min) * scale).ceil() as u32)
        .saturating_add(2)
        .max(1);
    let height = ((f32::from(bounds.y_max - bounds.y_min) * scale).ceil() as u32)
        .saturating_add(2)
        .max(1);
    let left =
        (pen_x + (x_offset_units as f32 + f32::from(bounds.x_min)) * scale).floor() as i32 - 1;
    let top =
        (baseline - (y_offset_units as f32 + f32::from(bounds.y_max)) * scale).floor() as i32 - 1;
    Some(GlyphPixelBounds {
        left,
        top,
        width,
        height,
    })
}

/// Renders a glyph whose pixel bounds are `bounds`, from `font`, the
/// stable identity of `bytes`.
pub(super) fn rasterize_glyph(
    font: &Arc<str>,
    bytes: &[u8],
    glyph_id: u16,
    font_size: f32,
    bounds: GlyphPixelBounds,
) -> Result<Option<GlyphBitmap>> {
    let key = (font.clone(), glyph_id, font_size.to_bits());
    let cached = glyph_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .glyphs
        .get(&key)
        .cloned();
    let alpha = match cached {
        Some(alpha) => alpha,
        None => {
            let alpha = rasterize_coverage(bytes, glyph_id, font_size, bounds)?;
            let mut cache = glyph_cache()
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let size = alpha.as_ref().map_or(0, |alpha| alpha.len());
            if cache.bytes + size > MAX_CACHED_GLYPH_BYTES {
                *cache = GlyphCache::default();
            }
            cache.bytes += size;
            cache.glyphs.insert(key, alpha.clone());
            alpha
        }
    };
    Ok(alpha.map(|alpha| GlyphBitmap {
        left: bounds.left,
        top: bounds.top,
        width: bounds.width,
        height: bounds.height,
        alpha,
    }))
}

fn rasterize_coverage(
    bytes: &[u8],
    glyph_id: u16,
    font_size: f32,
    bounds: GlyphPixelBounds,
) -> Result<Option<Arc<[u8]>>> {
    let face = Face::parse(bytes, 0).context("could not parse resolved text face")?;
    let glyph = GlyphId(glyph_id);
    let Some(outline_bounds) = face.glyph_bounding_box(glyph) else {
        return Ok(None);
    };
    let (width, height) = (bounds.width, bounds.height);
    let scale = font_size / f32::from(face.units_per_em());
    if u64::from(width) * u64::from(height) > MAX_GLYPH_PIXELS {
        bail!("shaped glyph exceeds the bounded rendering budget");
    }
    let mut builder = ScaledOutline::new(outline_bounds, scale);
    if face.outline_glyph(glyph, &mut builder).is_none() {
        return Ok(None);
    }
    let Some(path) = builder.finish() else {
        return Ok(None);
    };
    let mut pixmap =
        Pixmap::new(width, height).context("could not allocate shaped glyph bitmap")?;
    let mut paint = Paint::default();
    paint.set_color_rgba8(255, 255, 255, 255);
    pixmap.fill_path(
        &path,
        &paint,
        FillRule::Winding,
        Transform::identity(),
        None,
    );
    Ok(Some(
        pixmap.pixels().iter().map(|pixel| pixel.alpha()).collect(),
    ))
}

struct ScaledOutline {
    path: PathBuilder,
    bounds: Rect,
    scale: f32,
}

impl ScaledOutline {
    fn new(bounds: Rect, scale: f32) -> Self {
        Self {
            path: PathBuilder::new(),
            bounds,
            scale,
        }
    }

    fn point(&self, x: f32, y: f32) -> (f32, f32) {
        (
            (x - f32::from(self.bounds.x_min)) * self.scale + 1.0,
            (f32::from(self.bounds.y_max) - y) * self.scale + 1.0,
        )
    }

    fn finish(self) -> Option<tiny_skia::Path> {
        self.path.finish()
    }
}

impl OutlineBuilder for ScaledOutline {
    fn move_to(&mut self, x: f32, y: f32) {
        let (x, y) = self.point(x, y);
        self.path.move_to(x, y);
    }

    fn line_to(&mut self, x: f32, y: f32) {
        let (x, y) = self.point(x, y);
        self.path.line_to(x, y);
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let (x1, y1) = self.point(x1, y1);
        let (x, y) = self.point(x, y);
        self.path.quad_to(x1, y1, x, y);
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let (x1, y1) = self.point(x1, y1);
        let (x2, y2) = self.point(x2, y2);
        let (x, y) = self.point(x, y);
        self.path.cubic_to(x1, y1, x2, y2, x, y);
    }

    fn close(&mut self) {
        self.path.close();
    }
}
