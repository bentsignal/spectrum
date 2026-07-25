use anyhow::{Context, Result, bail};
use tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, Transform};
use ttf_parser::{Face, GlyphId, OutlineBuilder, Rect};

const MAX_GLYPH_PIXELS: u64 = 4_096 * 4_096;

pub(super) struct GlyphBitmap {
    pub(super) left: i32,
    pub(super) top: i32,
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) alpha: Vec<u8>,
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

pub(super) fn rasterize_glyph(
    bytes: &[u8],
    glyph_id: u16,
    font_size: f32,
    pen_x: f32,
    baseline: f32,
    x_offset_units: i32,
    y_offset_units: i32,
) -> Result<Option<GlyphBitmap>> {
    let face = Face::parse(bytes, 0).context("could not parse resolved text face")?;
    let glyph = GlyphId(glyph_id);
    let Some(bounds) = face.glyph_bounding_box(glyph) else {
        return Ok(None);
    };
    let pixel_bounds = glyph_pixel_bounds(
        &face,
        glyph_id,
        font_size,
        pen_x,
        baseline,
        x_offset_units,
        y_offset_units,
    )
    .expect("glyph bounds were resolved above");
    let width = pixel_bounds.width;
    let height = pixel_bounds.height;
    let scale = font_size / f32::from(face.units_per_em());
    if u64::from(width) * u64::from(height) > MAX_GLYPH_PIXELS {
        bail!("shaped glyph exceeds the bounded rendering budget");
    }
    let mut builder = ScaledOutline::new(bounds, scale);
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
    let alpha = pixmap.pixels().iter().map(|pixel| pixel.alpha()).collect();
    Ok(Some(GlyphBitmap {
        left: pixel_bounds.left,
        top: pixel_bounds.top,
        width,
        height,
        alpha,
    }))
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
