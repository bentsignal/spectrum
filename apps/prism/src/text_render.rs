use std::collections::VecDeque;

use anyhow::{Context, Result, bail};
use fontdue::Font;
use image::{Rgba, RgbaImage};

use crate::{FontAsset, RenderRegion, TextAlignment, TextTypography};

const MAX_EFFECT_PIXELS: u64 = 4_096 * 4_096;
const MAX_EFFECT_RADIUS: i32 = 2_048;
const MAX_EFFECT_OFFSET: i32 = 8_192;
const MAX_GLYPH_PIXELS: u64 = 4_096 * 4_096;

#[path = "text_render/font_cache.rs"]
mod font_cache;
use font_cache::cached_font;
#[cfg(test)]
pub(crate) use font_cache::font_outline_scale;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextGeometry {
    pub width: u32,
    pub height: u32,
    pub visual_left: f32,
    pub visual_top: f32,
    pub visual_width: f32,
    pub visual_height: f32,
    /// Logical paragraph layout bounds inside the returned image.
    pub layout_left: f32,
    pub layout_top: f32,
    pub layout_width: f32,
    pub layout_height: f32,
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
    measure_text_geometry_with_typography(text, font_size, &TextTypography::default(), None)
}

pub fn measure_text_with_typography(
    text: &str,
    font_size: f32,
    typography: &TextTypography,
    font_asset: Option<&FontAsset>,
) -> Result<(u32, u32)> {
    let geometry = measure_text_geometry_with_typography(text, font_size, typography, font_asset)?;
    Ok((geometry.width, geometry.height))
}

pub fn measure_text_geometry_with_typography(
    text: &str,
    font_size: f32,
    typography: &TextTypography,
    font_asset: Option<&FontAsset>,
) -> Result<TextGeometry> {
    let font = cached_font(font_asset, font_size)?;
    Ok(layout_text(&font, text, font_size, typography)?.geometry())
}

pub(crate) fn render_text(
    text: &str,
    font_size: f32,
    color: [u8; 4],
    typography: &TextTypography,
    font_asset: Option<&FontAsset>,
) -> Result<RgbaImage> {
    let font = cached_font(font_asset, font_size)?;
    let layout = layout_text(&font, text, font_size, typography)?;
    validate_effect_surface(&layout, typography)?;
    render_layout_region(
        &font,
        &layout,
        font_size,
        color,
        typography,
        RenderRegion {
            x: 0,
            y: 0,
            width: layout.width,
            height: layout.height,
        },
    )
}

pub(crate) fn render_text_region(
    text: &str,
    font_size: f32,
    color: [u8; 4],
    typography: &TextTypography,
    font_asset: Option<&FontAsset>,
    region: RenderRegion,
) -> Result<RgbaImage> {
    let font = cached_font(font_asset, font_size)?;
    let layout = layout_text(&font, text, font_size, typography)?;
    render_layout_region(&font, &layout, font_size, color, typography, region)
}

#[derive(Clone, Copy)]
struct PositionedGlyph {
    character: char,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

#[derive(Clone, Copy)]
struct PixelBounds {
    min_x: i32,
    min_y: i32,
    max_x: i32,
    max_y: i32,
}

impl PixelBounds {
    fn union(self, other: Self) -> Self {
        Self {
            min_x: self.min_x.min(other.min_x),
            min_y: self.min_y.min(other.min_y),
            max_x: self.max_x.max(other.max_x),
            max_y: self.max_y.max(other.max_y),
        }
    }

    fn expanded(self, amount: i32) -> Self {
        Self {
            min_x: self.min_x - amount,
            min_y: self.min_y - amount,
            max_x: self.max_x + amount,
            max_y: self.max_y + amount,
        }
    }

    fn translated(self, x: i32, y: i32) -> Self {
        Self {
            min_x: self.min_x + x,
            min_y: self.min_y + y,
            max_x: self.max_x + x,
            max_y: self.max_y + y,
        }
    }
}

struct TextLayout {
    glyphs: Vec<PositionedGlyph>,
    output_min_x: i32,
    output_min_y: i32,
    width: u32,
    height: u32,
    visual: PixelBounds,
    layout_box: PixelBounds,
}

impl TextLayout {
    fn geometry(&self) -> TextGeometry {
        TextGeometry {
            width: self.width,
            height: self.height,
            visual_left: (self.visual.min_x - self.output_min_x) as f32,
            visual_top: (self.visual.min_y - self.output_min_y) as f32,
            visual_width: (self.visual.max_x - self.visual.min_x).max(1) as f32,
            visual_height: (self.visual.max_y - self.visual.min_y).max(1) as f32,
            layout_left: (self.layout_box.min_x - self.output_min_x) as f32,
            layout_top: (self.layout_box.min_y - self.output_min_y) as f32,
            layout_width: (self.layout_box.max_x - self.layout_box.min_x).max(1) as f32,
            layout_height: (self.layout_box.max_y - self.layout_box.min_y).max(1) as f32,
        }
    }
}

fn layout_text(
    font: &Font,
    text: &str,
    font_size: f32,
    typography: &TextTypography,
) -> Result<TextLayout> {
    if !font_size.is_finite() || font_size <= 0.0 {
        bail!("text font size must be a positive finite number");
    }
    if !typography.line_height.is_finite()
        || typography.line_height <= 0.0
        || !typography.tracking.is_finite()
        || typography
            .box_width
            .is_some_and(|width| !width.is_finite() || width <= 0.0)
        || !typography.effects.outline_width.is_finite()
        || !typography.effects.shadow_offset_x.is_finite()
        || !typography.effects.shadow_offset_y.is_finite()
    {
        bail!("text typography contains invalid layout values");
    }
    let outline = typography.effects.outline_width.ceil() as i32;
    let shadow_x = typography.effects.shadow_offset_x.round() as i32;
    let shadow_y = typography.effects.shadow_offset_y.round() as i32;
    if outline > MAX_EFFECT_RADIUS {
        bail!("text outline exceeds the bounded rendering radius");
    }
    if shadow_x.abs() > MAX_EFFECT_OFFSET || shadow_y.abs() > MAX_EFFECT_OFFSET {
        bail!("text shadow exceeds the bounded rendering offset");
    }

    let line_metrics = font.horizontal_line_metrics(font_size);
    let ascent = line_metrics.map_or(font_size, |metrics| metrics.ascent);
    let natural_height = line_metrics.map_or(font_size, |metrics| metrics.new_line_size);
    let legacy_height = natural_height.max(font_size * 1.25).ceil().max(1.0);
    let line_height = (legacy_height * (typography.line_height / 1.25))
        .ceil()
        .max(1.0);
    let lines = wrapped_lines(font, text, font_size, typography);
    let advances = lines
        .iter()
        .map(|line| line_advance(font, line, font_size, typography.tracking))
        .collect::<Vec<_>>();
    let layout_width = typography
        .box_width
        .unwrap_or_else(|| advances.iter().copied().fold(1.0, f32::max))
        .max(1.0);
    let mut glyphs = Vec::new();
    let layout_box = PixelBounds {
        min_x: 0,
        min_y: 0,
        max_x: layout_width.ceil() as i32,
        max_y: (line_height * lines.len().max(1) as f32).ceil() as i32,
    };
    let mut logical = layout_box;
    let mut ink = None::<PixelBounds>;
    for (line_index, line) in lines.iter().enumerate() {
        let alignment_offset = match typography.alignment {
            TextAlignment::Left => 0.0,
            TextAlignment::Center => (layout_width - advances[line_index]) * 0.5,
            TextAlignment::Right => layout_width - advances[line_index],
        }
        .max(0.0);
        let mut cursor_x = alignment_offset;
        let count = line.chars().count();
        for (index, character) in line.chars().enumerate() {
            let metrics = font.metrics(character, font_size);
            let x = cursor_x.round() as i32 + metrics.xmin;
            let baseline = line_index as f32 * line_height + ascent;
            let y = baseline.round() as i32 - metrics.ymin - metrics.height as i32;
            let glyph = PositionedGlyph {
                character,
                x,
                y,
                width: metrics.width as u32,
                height: metrics.height as u32,
            };
            if glyph.width > 0 && glyph.height > 0 {
                let bounds = PixelBounds {
                    min_x: x,
                    min_y: y,
                    max_x: x + glyph.width as i32,
                    max_y: y + glyph.height as i32,
                };
                ink = Some(ink.map_or(bounds, |current| current.union(bounds)));
                logical = logical.union(bounds);
            }
            glyphs.push(glyph);
            cursor_x += metrics.advance_width;
            if index + 1 < count {
                cursor_x += typography.tracking;
            }
        }
        logical.max_x = logical.max_x.max(cursor_x.ceil() as i32);
    }

    let base_visual = ink.unwrap_or(logical);
    let mut visual = base_visual;
    if outline > 0 && typography.effects.outline_color[3] > 0 {
        visual = visual.union(base_visual.expanded(outline));
    }
    if typography.effects.shadow_color[3] > 0 {
        visual = visual.union(base_visual.translated(shadow_x, shadow_y));
    }
    let output = logical.union(visual);
    let width = u32::try_from((output.max_x - output.min_x).max(1))
        .context("text width exceeds the supported range")?;
    let height = u32::try_from((output.max_y - output.min_y).max(1))
        .context("text height exceeds the supported range")?;
    Ok(TextLayout {
        glyphs,
        output_min_x: output.min_x,
        output_min_y: output.min_y,
        width,
        height,
        visual,
        layout_box,
    })
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
        let mut width = 0.0;
        let mut count = 0_usize;
        for character in paragraph.chars() {
            let advance = font.metrics(character, font_size).advance_width;
            let added = advance + if count > 0 { typography.tracking } else { 0.0 };
            if count > 0 && width + added > limit {
                let split = line
                    .char_indices()
                    .rev()
                    .find_map(|(index, value)| value.is_whitespace().then_some(index));
                if let Some(index) = split {
                    let remainder = line[index..].trim_start().to_owned();
                    let completed = line[..index].trim_end().to_owned();
                    if !completed.is_empty() {
                        output.push(completed);
                    }
                    line = remainder;
                    count = line.chars().count();
                    width = line_advance(font, &line, font_size, typography.tracking);
                } else {
                    output.push(line);
                    line = String::new();
                    count = 0;
                    width = 0.0;
                }
                let added = advance + if count > 0 { typography.tracking } else { 0.0 };
                if count > 0 && width + added > limit {
                    output.push(line);
                    line = String::new();
                    count = 0;
                    width = 0.0;
                }
            }
            if count > 0 {
                width += typography.tracking;
            }
            line.push(character);
            width += advance;
            count += 1;
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

fn validate_effect_surface(layout: &TextLayout, typography: &TextTypography) -> Result<()> {
    let effects_active = (typography.effects.outline_width > 0.0
        && typography.effects.outline_color[3] > 0)
        || typography.effects.shadow_color[3] > 0;
    if effects_active && u64::from(layout.width) * u64::from(layout.height) > MAX_EFFECT_PIXELS {
        bail!("text effects exceed the bounded rendering surface budget");
    }
    Ok(())
}

fn render_layout_region(
    font: &Font,
    layout: &TextLayout,
    font_size: f32,
    color: [u8; 4],
    typography: &TextTypography,
    region: RenderRegion,
) -> Result<RgbaImage> {
    let right = region
        .x
        .checked_add(region.width)
        .context("text region overflows")?;
    let bottom = region
        .y
        .checked_add(region.height)
        .context("text region overflows")?;
    if right > layout.width || bottom > layout.height {
        bail!("text render region exceeds the layout bounds");
    }
    let mut output = RgbaImage::new(region.width, region.height);
    let effects = typography.effects;
    if effects.shadow_color[3] > 0 {
        paint_glyphs(
            &mut output,
            font,
            layout,
            font_size,
            region,
            effects.shadow_color,
            effects.shadow_offset_x.round() as i32,
            effects.shadow_offset_y.round() as i32,
        )?;
    }
    let radius = effects.outline_width.ceil() as i32;
    if radius > 0 && effects.outline_color[3] > 0 {
        paint_outline(
            &mut output,
            font,
            layout,
            font_size,
            region,
            radius,
            effects.outline_color,
        )?;
    }
    paint_glyphs(&mut output, font, layout, font_size, region, color, 0, 0)?;
    Ok(output)
}

#[allow(clippy::too_many_arguments)]
fn paint_glyphs(
    output: &mut RgbaImage,
    font: &Font,
    layout: &TextLayout,
    font_size: f32,
    region: RenderRegion,
    color: [u8; 4],
    shift_x: i32,
    shift_y: i32,
) -> Result<()> {
    let region_right = i64::from(region.x + region.width);
    let region_bottom = i64::from(region.y + region.height);
    for glyph in &layout.glyphs {
        let left = i64::from(glyph.x - layout.output_min_x + shift_x);
        let top = i64::from(glyph.y - layout.output_min_y + shift_y);
        if left + i64::from(glyph.width) <= i64::from(region.x)
            || top + i64::from(glyph.height) <= i64::from(region.y)
            || left >= region_right
            || top >= region_bottom
        {
            continue;
        }
        if u64::from(glyph.width) * u64::from(glyph.height) > MAX_GLYPH_PIXELS {
            bail!("text glyph exceeds the bounded rendering budget");
        }
        let (_, bitmap) = font.rasterize(glyph.character, font_size);
        for row in 0..glyph.height {
            for column in 0..glyph.width {
                let x = left + i64::from(column) - i64::from(region.x);
                let y = top + i64::from(row) - i64::from(region.y);
                if x >= 0 && y >= 0 && x < i64::from(region.width) && y < i64::from(region.height) {
                    let coverage = bitmap[row as usize * glyph.width as usize + column as usize];
                    let alpha = u16::from(coverage) * u16::from(color[3]) / 255;
                    composite_over(
                        output,
                        x as u32,
                        y as u32,
                        [color[0], color[1], color[2], alpha as u8],
                    );
                }
            }
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn paint_outline(
    output: &mut RgbaImage,
    font: &Font,
    layout: &TextLayout,
    font_size: f32,
    region: RenderRegion,
    radius: i32,
    color: [u8; 4],
) -> Result<()> {
    let expanded = expanded_region(region, layout.width, layout.height, radius as u32);
    if region_pixel_count(expanded) > MAX_EFFECT_PIXELS {
        bail!("text outline exceeds the bounded staging budget");
    }
    let mut alpha = vec![0_u8; region_pixel_count(expanded) as usize];
    rasterize_alpha(font, layout, font_size, expanded, &mut alpha)?;
    let alpha = dilate_alpha(&alpha, expanded.width, expanded.height, radius as u32);
    let offset_x = region.x - expanded.x;
    let offset_y = region.y - expanded.y;
    for y in 0..region.height {
        for x in 0..region.width {
            let coverage =
                alpha[(y + offset_y) as usize * expanded.width as usize + (x + offset_x) as usize];
            let alpha = u16::from(coverage) * u16::from(color[3]) / 255;
            composite_over(output, x, y, [color[0], color[1], color[2], alpha as u8]);
        }
    }
    Ok(())
}

fn region_pixel_count(region: RenderRegion) -> u64 {
    u64::from(region.width) * u64::from(region.height)
}

fn expanded_region(region: RenderRegion, width: u32, height: u32, radius: u32) -> RenderRegion {
    let x = region.x.saturating_sub(radius);
    let y = region.y.saturating_sub(radius);
    let right = region
        .x
        .saturating_add(region.width)
        .saturating_add(radius)
        .min(width);
    let bottom = region
        .y
        .saturating_add(region.height)
        .saturating_add(radius)
        .min(height);
    RenderRegion {
        x,
        y,
        width: right - x,
        height: bottom - y,
    }
}

fn rasterize_alpha(
    font: &Font,
    layout: &TextLayout,
    font_size: f32,
    region: RenderRegion,
    alpha: &mut [u8],
) -> Result<()> {
    for glyph in &layout.glyphs {
        let left = glyph.x - layout.output_min_x;
        let top = glyph.y - layout.output_min_y;
        if left + glyph.width as i32 <= region.x as i32
            || top + glyph.height as i32 <= region.y as i32
            || left >= (region.x + region.width) as i32
            || top >= (region.y + region.height) as i32
        {
            continue;
        }
        if u64::from(glyph.width) * u64::from(glyph.height) > MAX_GLYPH_PIXELS {
            bail!("text glyph exceeds the bounded rendering budget");
        }
        let (_, bitmap) = font.rasterize(glyph.character, font_size);
        for row in 0..glyph.height {
            for column in 0..glyph.width {
                let x = left + column as i32 - region.x as i32;
                let y = top + row as i32 - region.y as i32;
                if x >= 0 && y >= 0 && x < region.width as i32 && y < region.height as i32 {
                    let target = y as usize * region.width as usize + x as usize;
                    alpha[target] = alpha[target]
                        .max(bitmap[row as usize * glyph.width as usize + column as usize]);
                }
            }
        }
    }
    Ok(())
}

fn dilate_alpha(source: &[u8], width: u32, height: u32, radius: u32) -> Vec<u8> {
    let mut horizontal = vec![0; source.len()];
    for y in 0..height as usize {
        max_filter_line(
            source,
            &mut horizontal,
            y * width as usize,
            1,
            width as usize,
            radius as usize,
        );
    }
    let mut output = vec![0; source.len()];
    for x in 0..width as usize {
        max_filter_line(
            &horizontal,
            &mut output,
            x,
            width as usize,
            height as usize,
            radius as usize,
        );
    }
    output
}

fn max_filter_line(
    source: &[u8],
    output: &mut [u8],
    offset: usize,
    stride: usize,
    length: usize,
    radius: usize,
) {
    let mut queue = VecDeque::<(usize, u8)>::new();
    let mut next = 0;
    for center in 0..length {
        let right = center.saturating_add(radius).min(length.saturating_sub(1));
        while next <= right {
            let value = source[offset + next * stride];
            while queue.back().is_some_and(|(_, current)| *current <= value) {
                queue.pop_back();
            }
            queue.push_back((next, value));
            next += 1;
        }
        let left = center.saturating_sub(radius);
        while queue.front().is_some_and(|(index, _)| *index < left) {
            queue.pop_front();
        }
        output[offset + center * stride] = queue.front().map_or(0, |(_, value)| *value);
    }
}

fn composite_over(image: &mut RgbaImage, x: u32, y: u32, source: [u8; 4]) {
    if source[3] == 0 {
        return;
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TextEffects;
    use std::path::PathBuf;

    #[test]
    fn text_geometry_uses_visible_glyph_bounds_inside_the_line_box() {
        let geometry = measure_text_geometry("I", 72.0).unwrap();
        assert!(geometry.visual_width < geometry.width as f32);
        assert!(geometry.visual_height < geometry.height as f32);
        assert!(geometry.visual_center().0 > geometry.visual_left);
        assert!(geometry.visual_center().1 > geometry.visual_top);
    }

    #[test]
    fn paragraph_metrics_change_wrapping_alignment_tracking_and_effect_bounds() {
        let base = TextTypography {
            box_width: Some(150.0),
            ..Default::default()
        };
        let unwrapped = measure_text_geometry_with_typography(
            "one two three",
            32.0,
            &TextTypography::default(),
            None,
        )
        .unwrap();
        let left =
            measure_text_geometry_with_typography("one two three", 32.0, &base, None).unwrap();
        let changed = TextTypography {
            alignment: TextAlignment::Right,
            line_height: 1.8,
            tracking: 3.0,
            effects: TextEffects {
                outline_width: 4.0,
                shadow_offset_x: -7.0,
                shadow_offset_y: 9.0,
                shadow_color: [0, 0, 0, 180],
                ..Default::default()
            },
            ..base
        };
        let right =
            measure_text_geometry_with_typography("one two three", 32.0, &changed, None).unwrap();
        assert!(left.height > unwrapped.height);
        assert!(right.height > left.height);
        assert!(right.visual_left != left.visual_left);
        assert!(right.visual_width > left.visual_width);
        assert!(right.visual_height > left.visual_height);
    }

    #[test]
    fn parsed_imported_fonts_are_reused_by_content_hash() {
        let path = temporary_font("cache");
        let asset = FontAsset::import(7, &path).unwrap();
        let first = cached_font(Some(&asset), 72.0).unwrap();
        let second = cached_font(Some(&asset), 72.0).unwrap();
        let _ = std::fs::remove_file(path);
        assert!(std::sync::Arc::ptr_eq(&first, &second));
    }

    #[test]
    fn outline_quality_uses_bounded_power_of_two_tiers() {
        assert_eq!(font_outline_scale(4.0), 64);
        assert_eq!(font_outline_scale(64.0), 64);
        assert_eq!(font_outline_scale(64.1), 128);
        assert_eq!(font_outline_scale(768.0), 1_024);
        assert_eq!(font_outline_scale(16_000.0), 4_096);
        assert_eq!(font_outline_scale(f32::NAN), 64);
    }

    #[test]
    fn large_curves_match_high_resolution_oracles_for_bundled_and_imported_fonts() {
        assert_large_curve_matches_oracle(None, epaint_default_fonts::UBUNTU_LIGHT);

        let imported = include_bytes!(
            "../../../crates/spectrum-fonts/tests/fonts/noto-sans-static-source.ttf"
        );
        let path = temporary_font_bytes("large-curve-oracle", imported);
        let asset = FontAsset::import(17, &path).unwrap();
        let bytes = asset.bytes().unwrap();
        assert_large_curve_matches_oracle(Some(&asset), &bytes);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn outline_dilation_cost_is_independent_of_radius_squared() {
        let source = [0, 0, 255, 0, 0];
        assert_eq!(dilate_alpha(&source, 5, 1, 2), vec![255; 5]);
    }

    #[test]
    fn outline_and_shadow_are_painted_and_match_region_crops() {
        let typography = TextTypography {
            effects: TextEffects {
                outline_width: 3.0,
                outline_color: [19, 71, 137, 255],
                shadow_offset_x: 11.0,
                shadow_offset_y: 9.0,
                shadow_color: [143, 47, 23, 255],
            },
            ..Default::default()
        };
        let full = render_text("Effects", 44.0, [238, 231, 207, 255], &typography, None).unwrap();
        assert!(full.pixels().any(|pixel| pixel.0[..3] == [19, 71, 137]));
        assert!(full.pixels().any(|pixel| pixel.0[..3] == [143, 47, 23]));
        let region = RenderRegion {
            x: full.width() / 4,
            y: full.height() / 4,
            width: full.width() / 2,
            height: full.height() / 2,
        };
        let staged = render_text_region(
            "Effects",
            44.0,
            [238, 231, 207, 255],
            &typography,
            None,
            region,
        )
        .unwrap();
        let oracle =
            image::imageops::crop_imm(&full, region.x, region.y, region.width, region.height)
                .to_image();
        assert_eq!(staged, oracle);
    }

    #[test]
    fn excessive_scaled_effects_are_rejected_before_raster_allocation() {
        let typography = TextTypography {
            effects: TextEffects {
                outline_width: (MAX_EFFECT_RADIUS + 1) as f32,
                ..Default::default()
            },
            ..Default::default()
        };
        let error =
            measure_text_geometry_with_typography("Bounded", 48.0, &typography, None).unwrap_err();
        assert!(error.to_string().contains("bounded rendering radius"));
    }

    fn temporary_font(label: &str) -> PathBuf {
        temporary_font_bytes(label, epaint_default_fonts::HACK_REGULAR)
    }

    fn temporary_font_bytes(label: &str, bytes: &[u8]) -> PathBuf {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let temporary_root = std::env::temp_dir();
        let path = std::fs::canonicalize(&temporary_root)
            .unwrap_or(temporary_root)
            .join(format!("prism-{label}-{stamp}.ttf"));
        std::fs::write(&path, bytes).unwrap();
        path
    }

    fn assert_large_curve_matches_oracle(font_asset: Option<&FontAsset>, bytes: &[u8]) {
        let font_size = 768.0;
        let actual_font = cached_font(font_asset, font_size).unwrap();
        let (_, actual) = actual_font.rasterize('S', font_size);
        let oracle_font = Font::from_bytes(
            bytes,
            fontdue::FontSettings {
                scale: font_outline_scale(font_size) as f32,
                ..fontdue::FontSettings::default()
            },
        )
        .unwrap();
        let (_, oracle) = oracle_font.rasterize('S', font_size);
        assert_eq!(actual, oracle);

        let low_detail_font = Font::from_bytes(bytes, fontdue::FontSettings::default()).unwrap();
        let (_, low_detail) = low_detail_font.rasterize('S', font_size);
        assert_eq!(actual.len(), low_detail.len());
        let changed_pixels = actual
            .iter()
            .zip(low_detail)
            .filter(|(actual, low_detail)| actual != &low_detail)
            .count();
        assert!(
            changed_pixels > 1_000,
            "large curved glyph should materially differ from fontdue's 40 px outline tier"
        );
    }
}
