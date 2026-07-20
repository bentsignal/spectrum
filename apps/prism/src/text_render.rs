use anyhow::{Result, bail};
use fontdue::Font;
use image::{Rgba, RgbaImage};

use crate::RenderRegion;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextGeometry {
    pub width: u32,
    pub height: u32,
    pub visual_left: f32,
    pub visual_top: f32,
    pub visual_width: f32,
    pub visual_height: f32,
}

impl TextGeometry {
    pub fn visual_center(self) -> (f32, f32) {
        (
            self.visual_left + self.visual_width * 0.5,
            self.visual_top + self.visual_height * 0.5,
        )
    }
}

pub fn measure_text(text: &str, font_size: f32) -> Result<(u32, u32)> {
    let geometry = measure_text_geometry(text, font_size)?;
    Ok((geometry.width, geometry.height))
}

pub fn measure_text_geometry(text: &str, font_size: f32) -> Result<TextGeometry> {
    let font = bundled_font()?;
    Ok(layout_text(&font, text, font_size).geometry())
}

pub(crate) fn render_text(text: &str, font_size: f32, color: [u8; 4]) -> Result<RgbaImage> {
    let font = bundled_font()?;
    let layout = layout_text(&font, text, font_size);
    let mut output = RgbaImage::new(layout.width, layout.height);
    let (width, height) = output.dimensions();
    rasterize_intersecting_glyphs(
        &font,
        layout,
        font_size,
        color,
        RenderRegion {
            x: 0,
            y: 0,
            width,
            height,
        },
        &mut output,
        false,
    )?;
    Ok(output)
}

pub(crate) fn render_text_region(
    text: &str,
    font_size: f32,
    color: [u8; 4],
    region: RenderRegion,
) -> Result<RgbaImage> {
    let font = bundled_font()?;
    let layout = layout_text(&font, text, font_size);
    let mut output = RgbaImage::new(region.width, region.height);
    rasterize_intersecting_glyphs(&font, layout, font_size, color, region, &mut output, true)?;
    Ok(output)
}

fn bundled_font() -> Result<Font> {
    Font::from_bytes(
        epaint_default_fonts::UBUNTU_LIGHT,
        fontdue::FontSettings::default(),
    )
    .map_err(|error| anyhow::anyhow!("could not load bundled font: {error}"))
}

#[allow(clippy::too_many_arguments)]
fn rasterize_intersecting_glyphs(
    font: &Font,
    layout: TextLayout,
    font_size: f32,
    color: [u8; 4],
    region: RenderRegion,
    output: &mut RgbaImage,
    enforce_glyph_budget: bool,
) -> Result<()> {
    const MAX_GLYPH_PIXELS: u64 = 4_096 * 4_096;
    for glyph in layout.glyphs {
        let metrics = font.metrics(glyph.character, font_size);
        let left = glyph.x - layout.min_x;
        let top = glyph.y - layout.min_y;
        if left + metrics.width as i32 <= region.x as i32
            || top + metrics.height as i32 <= region.y as i32
            || left >= (region.x + region.width) as i32
            || top >= (region.y + region.height) as i32
        {
            continue;
        }
        if enforce_glyph_budget
            && (metrics.width as u64) * (metrics.height as u64) > MAX_GLYPH_PIXELS
        {
            bail!("text glyph exceeds the bounded viewport staging budget");
        }
        let (_, bitmap) = font.rasterize(glyph.character, font_size);
        for row in 0..metrics.height {
            for column in 0..metrics.width {
                let x = left + column as i32 - region.x as i32;
                let y = top + row as i32 - region.y as i32;
                if x >= 0 && y >= 0 && x < region.width as i32 && y < region.height as i32 {
                    let alpha = bitmap[row * metrics.width + column] as u16 * color[3] as u16 / 255;
                    output.put_pixel(
                        x as u32,
                        y as u32,
                        Rgba([color[0], color[1], color[2], alpha as u8]),
                    );
                }
            }
        }
    }
    Ok(())
}

struct PositionedGlyph {
    character: char,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

struct TextLayout {
    glyphs: Vec<PositionedGlyph>,
    min_x: i32,
    min_y: i32,
    width: u32,
    height: u32,
}

impl TextLayout {
    fn geometry(&self) -> TextGeometry {
        let visual = self
            .glyphs
            .iter()
            .filter(|glyph| glyph.width > 0 && glyph.height > 0)
            .fold(None::<(i32, i32, i32, i32)>, |bounds, glyph| {
                let left = glyph.x - self.min_x;
                let top = glyph.y - self.min_y;
                let right = left + glyph.width as i32;
                let bottom = top + glyph.height as i32;
                Some(match bounds {
                    Some((min_x, min_y, max_x, max_y)) => (
                        min_x.min(left),
                        min_y.min(top),
                        max_x.max(right),
                        max_y.max(bottom),
                    ),
                    None => (left, top, right, bottom),
                })
            })
            .unwrap_or((0, 0, self.width as i32, self.height as i32));
        TextGeometry {
            width: self.width,
            height: self.height,
            visual_left: visual.0 as f32,
            visual_top: visual.1 as f32,
            visual_width: (visual.2 - visual.0).max(1) as f32,
            visual_height: (visual.3 - visual.1).max(1) as f32,
        }
    }
}

fn layout_text(font: &Font, text: &str, font_size: f32) -> TextLayout {
    let line_metrics = font.horizontal_line_metrics(font_size);
    let ascent = line_metrics.map_or(font_size, |metrics| metrics.ascent);
    let natural_height = line_metrics.map_or(font_size, |metrics| metrics.new_line_size);
    let line_height = natural_height.max(font_size * 1.25).ceil().max(1.0);
    let lines: Vec<_> = text.split('\n').collect();
    let mut glyphs = Vec::new();
    let mut min_x = 0;
    let mut min_y = 0;
    let mut max_x = 1;
    let mut max_y = (line_height * lines.len().max(1) as f32).ceil() as i32;
    for (line_index, line) in lines.iter().enumerate() {
        let mut cursor_x: f32 = 0.0;
        for character in line.chars() {
            let metrics = font.metrics(character, font_size);
            let x = cursor_x.round() as i32 + metrics.xmin;
            let baseline = line_index as f32 * line_height + ascent;
            let y = baseline.round() as i32 - metrics.ymin - metrics.height as i32;
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x + metrics.width as i32);
            max_y = max_y.max(y + metrics.height as i32);
            glyphs.push(PositionedGlyph {
                character,
                x,
                y,
                width: metrics.width as u32,
                height: metrics.height as u32,
            });
            cursor_x += metrics.advance_width;
        }
        max_x = max_x.max(cursor_x.ceil() as i32);
    }
    TextLayout {
        glyphs,
        min_x,
        min_y,
        width: (max_x - min_x).max(1) as u32,
        height: (max_y - min_y).max(1) as u32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_geometry_uses_visible_glyph_bounds_inside_the_line_box() {
        let geometry = measure_text_geometry("I", 72.0).unwrap();
        assert!(geometry.visual_width < geometry.width as f32);
        assert!(geometry.visual_height < geometry.height as f32);
        assert!(geometry.visual_center().0 > geometry.visual_left);
        assert!(geometry.visual_center().1 > geometry.visual_top);
    }
}
