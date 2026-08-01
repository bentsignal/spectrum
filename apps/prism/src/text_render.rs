use anyhow::{Result, anyhow};
use fontdue::Font;
use image::{Rgba, RgbaImage};

use crate::TextTypography;

pub(crate) fn measure_text(
    text: &str,
    font_size: f32,
    typography: &TextTypography,
    font_data: Option<&[u8]>,
) -> Result<(u32, u32)> {
    let font = load_font(font_data)?;
    let layout = layout_text(&font, text, font_size, typography);
    Ok(layout.output_size(typography))
}

pub(crate) fn render_text(
    text: &str,
    font_size: f32,
    color: [u8; 4],
    typography: &TextTypography,
    font_data: Option<&[u8]>,
) -> Result<RgbaImage> {
    let font = load_font(font_data)?;
    let layout = layout_text(&font, text, font_size, typography);
    let (width, height) = layout.output_size(typography);
    let margin = effect_margin(typography);
    let mut foreground = RgbaImage::new(width, height);
    for glyph in &layout.glyphs {
        let (metrics, bitmap) = font.rasterize(glyph.character, font_size);
        for row in 0..metrics.height {
            for column in 0..metrics.width {
                let x = glyph.x + column as i32 - layout.min_x + margin.left;
                let y = glyph.y + row as i32 - layout.min_y + margin.top;
                if x >= 0 && y >= 0 && (x as u32) < width && (y as u32) < height {
                    let alpha = bitmap[row * metrics.width + column] as u16 * color[3] as u16 / 255;
                    foreground.put_pixel(
                        x as u32,
                        y as u32,
                        Rgba([color[0], color[1], color[2], alpha as u8]),
                    );
                }
            }
        }
    }
    if typography.effects == Default::default() {
        return Ok(foreground);
    }

    let mut output = RgbaImage::new(width, height);
    paint_shadow(&mut output, &foreground, typography);
    paint_outline(&mut output, &foreground, typography);
    for (x, y, pixel) in foreground.enumerate_pixels() {
        composite_over(&mut output, x, y, pixel.0);
    }
    Ok(output)
}

fn load_font(font_data: Option<&[u8]>) -> Result<Font> {
    Font::from_bytes(
        font_data.unwrap_or(epaint_default_fonts::UBUNTU_LIGHT),
        fontdue::FontSettings::default(),
    )
    .map_err(|error| anyhow!("could not load text font: {error}"))
}

struct PositionedGlyph {
    character: char,
    x: i32,
    y: i32,
}

struct TextLayout {
    glyphs: Vec<PositionedGlyph>,
    min_x: i32,
    min_y: i32,
    width: u32,
    height: u32,
}

impl TextLayout {
    fn output_size(&self, typography: &TextTypography) -> (u32, u32) {
        let margin = effect_margin(typography);
        (
            self.width
                .saturating_add((margin.left + margin.right).max(0) as u32),
            self.height
                .saturating_add((margin.top + margin.bottom).max(0) as u32),
        )
    }
}

fn layout_text(font: &Font, text: &str, font_size: f32, typography: &TextTypography) -> TextLayout {
    let line_metrics = font.horizontal_line_metrics(font_size);
    let ascent = line_metrics.map_or(font_size, |metrics| metrics.ascent);
    let natural_height = line_metrics.map_or(font_size, |metrics| metrics.new_line_size);
    let legacy_line_height = natural_height.max(font_size * 1.25).ceil().max(1.0);
    let line_height = (legacy_line_height * (typography.line_height / 1.25))
        .ceil()
        .max(1.0);
    let lines = wrapped_lines(font, text, font_size, typography);
    let measured: Vec<_> = lines
        .iter()
        .map(|line| line_advance(font, line, font_size, typography.tracking))
        .collect();
    let layout_width = typography
        .box_width
        .unwrap_or_else(|| measured.iter().copied().fold(1.0, f32::max))
        .max(1.0);
    let mut glyphs = Vec::new();
    let mut min_x = 0;
    let mut min_y = 0;
    let mut max_x = layout_width.ceil() as i32;
    let mut max_y = (line_height * lines.len().max(1) as f32).ceil() as i32;
    for (line_index, line) in lines.iter().enumerate() {
        let alignment_offset = match typography.alignment {
            crate::TextAlignment::Left => 0.0,
            crate::TextAlignment::Center => (layout_width - measured[line_index]) * 0.5,
            crate::TextAlignment::Right => layout_width - measured[line_index],
        }
        .max(0.0);
        let mut cursor_x = alignment_offset;
        let character_count = line.chars().count();
        for (index, character) in line.chars().enumerate() {
            let metrics = font.metrics(character, font_size);
            let x = cursor_x.round() as i32 + metrics.xmin;
            let baseline = line_index as f32 * line_height + ascent;
            let y = baseline.round() as i32 - metrics.ymin - metrics.height as i32;
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x + metrics.width as i32);
            max_y = max_y.max(y + metrics.height as i32);
            glyphs.push(PositionedGlyph { character, x, y });
            cursor_x += metrics.advance_width;
            if index + 1 < character_count {
                cursor_x += typography.tracking;
            }
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

fn wrapped_lines(
    font: &Font,
    text: &str,
    font_size: f32,
    typography: &TextTypography,
) -> Vec<String> {
    let Some(limit) = typography.box_width else {
        return text.split('\n').map(str::to_owned).collect();
    };
    let mut output = Vec::new();
    for paragraph in text.split('\n') {
        if paragraph.is_empty() {
            output.push(String::new());
            continue;
        }
        let mut line = String::new();
        for word in paragraph.split_inclusive(char::is_whitespace) {
            let candidate = format!("{line}{word}");
            if !line.is_empty()
                && line_advance(font, &candidate, font_size, typography.tracking) > limit
            {
                output.push(line.trim_end().to_owned());
                line = word.trim_start().to_owned();
            } else {
                line.push_str(word);
            }
        }
        output.push(line.trim_end().to_owned());
    }
    if output.is_empty() {
        output.push(String::new());
    }
    output
}

fn line_advance(font: &Font, line: &str, font_size: f32, tracking: f32) -> f32 {
    let count = line.chars().count();
    line.chars()
        .map(|character| font.metrics(character, font_size).advance_width)
        .sum::<f32>()
        + tracking * count.saturating_sub(1) as f32
}

#[derive(Clone, Copy)]
struct EffectMargin {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

fn effect_margin(typography: &TextTypography) -> EffectMargin {
    let outline = typography.effects.outline_width.ceil() as i32;
    let shadow_x = typography.effects.shadow_offset_x.round() as i32;
    let shadow_y = typography.effects.shadow_offset_y.round() as i32;
    EffectMargin {
        left: outline + (-shadow_x).max(0),
        top: outline + (-shadow_y).max(0),
        right: outline + shadow_x.max(0),
        bottom: outline + shadow_y.max(0),
    }
}

fn paint_shadow(output: &mut RgbaImage, foreground: &RgbaImage, typography: &TextTypography) {
    let color = typography.effects.shadow_color;
    if color[3] == 0 {
        return;
    }
    let dx = typography.effects.shadow_offset_x.round() as i64;
    let dy = typography.effects.shadow_offset_y.round() as i64;
    for (x, y, pixel) in foreground
        .enumerate_pixels()
        .filter(|(_, _, pixel)| pixel[3] > 0)
    {
        let target_x = i64::from(x) + dx;
        let target_y = i64::from(y) + dy;
        if target_x >= 0
            && target_y >= 0
            && target_x < i64::from(output.width())
            && target_y < i64::from(output.height())
        {
            let alpha = u16::from(pixel[3]) * u16::from(color[3]) / 255;
            composite_over(
                output,
                target_x as u32,
                target_y as u32,
                [color[0], color[1], color[2], alpha as u8],
            );
        }
    }
}

fn paint_outline(output: &mut RgbaImage, foreground: &RgbaImage, typography: &TextTypography) {
    let radius = typography.effects.outline_width.ceil() as i32;
    if radius <= 0 || typography.effects.outline_color[3] == 0 {
        return;
    }
    let color = typography.effects.outline_color;
    let mut coverage = vec![0_u8; output.width() as usize * output.height() as usize];
    for (x, y, pixel) in foreground
        .enumerate_pixels()
        .filter(|(_, _, pixel)| pixel[3] > 0)
    {
        for dy in -radius..=radius {
            for dx in -radius..=radius {
                if dx * dx + dy * dy > radius * radius {
                    continue;
                }
                let target_x = i64::from(x) + i64::from(dx);
                let target_y = i64::from(y) + i64::from(dy);
                if target_x >= 0
                    && target_y >= 0
                    && target_x < i64::from(output.width())
                    && target_y < i64::from(output.height())
                {
                    let index = target_y as usize * output.width() as usize + target_x as usize;
                    coverage[index] = coverage[index].max(pixel[3]);
                }
            }
        }
    }
    for (index, coverage_alpha) in coverage.into_iter().enumerate() {
        if coverage_alpha == 0 {
            continue;
        }
        let alpha = u16::from(coverage_alpha) * u16::from(color[3]) / 255;
        composite_over(
            output,
            (index % output.width() as usize) as u32,
            (index / output.width() as usize) as u32,
            [color[0], color[1], color[2], alpha as u8],
        );
    }
}

fn composite_over(image: &mut RgbaImage, x: u32, y: u32, source: [u8; 4]) {
    let destination = image.get_pixel(x, y).0;
    let source_alpha = f32::from(source[3]) / 255.0;
    let destination_alpha = f32::from(destination[3]) / 255.0;
    let output_alpha = source_alpha + destination_alpha * (1.0 - source_alpha);
    let mut output = [0; 4];
    for channel in 0..3 {
        output[channel] = if output_alpha > 0.0 {
            ((f32::from(source[channel]) * source_alpha
                + f32::from(destination[channel]) * destination_alpha * (1.0 - source_alpha))
                / output_alpha)
                .round() as u8
        } else {
            0
        };
    }
    output[3] = (output_alpha * 255.0).round() as u8;
    image.put_pixel(x, y, Rgba(output));
}
