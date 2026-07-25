use std::{collections::VecDeque, ops::Range};

use anyhow::{Context, Result, bail};
use image::{Rgba, RgbaImage};
use spectrum_fonts::{HarfBuzzShaper, Script, ShapeRequest, TextDirection};
use unicode_bidi::BidiInfo;
use unicode_linebreak::{BreakOpportunity, linebreaks};
use unicode_script::{Script as UnicodeScript, UnicodeScript as _};
use unicode_segmentation::UnicodeSegmentation;

use super::{
    font_resolver::{FaceChoice, ResolvedFonts, validate_grapheme_boundaries},
    glyph_raster::{GlyphBitmap, glyph_pixel_bounds, rasterize_glyph},
    legacy::TextGeometry,
};
use crate::{FontAsset, RenderRegion, TextAlignment, TextTypography};

const MAX_LAYOUT_PIXELS: u64 = 4_096 * 4_096;
const MAX_BREAK_OPPORTUNITIES: usize = 4_096;
const MAX_EFFECT_RADIUS: i32 = 2_048;
const MAX_EFFECT_OFFSET: i32 = 8_192;

#[derive(Clone)]
struct PositionedGlyph {
    face: FaceChoice,
    glyph_id: u16,
    pen_x: f32,
    baseline: f32,
    x_offset: i32,
    y_offset: i32,
}

#[derive(Clone, Copy)]
struct Bounds {
    min_x: i32,
    min_y: i32,
    max_x: i32,
    max_y: i32,
}

impl Bounds {
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
    fonts: ResolvedFonts,
    font_size: f32,
    glyphs: Vec<PositionedGlyph>,
    output_min_x: i32,
    output_min_y: i32,
    width: u32,
    height: u32,
    visual: Bounds,
    layout_box: Bounds,
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

#[derive(Clone)]
struct RawGlyph {
    face: FaceChoice,
    glyph_id: u16,
    cluster: u32,
    cluster_end: u32,
    units_per_em: u16,
    x_advance: i32,
    x_offset: i32,
    y_offset: i32,
    ascender: i16,
    descender: i16,
    line_gap: i16,
}

pub(super) fn measure(
    text: &str,
    font_size: f32,
    typography: &TextTypography,
    font_asset: Option<&FontAsset>,
) -> Result<TextGeometry> {
    Ok(layout_text(text, font_size, typography, font_asset)?.geometry())
}

pub(super) fn render(
    text: &str,
    font_size: f32,
    color: [u8; 4],
    typography: &TextTypography,
    font_asset: Option<&FontAsset>,
) -> Result<RgbaImage> {
    let layout = layout_text(text, font_size, typography, font_asset)?;
    render_layout_region(
        &layout,
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

pub(super) fn render_region(
    text: &str,
    font_size: f32,
    color: [u8; 4],
    typography: &TextTypography,
    font_asset: Option<&FontAsset>,
    region: RenderRegion,
) -> Result<RgbaImage> {
    let layout = layout_text(text, font_size, typography, font_asset)?;
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
    render_layout_region(&layout, color, typography, region)
}

fn layout_text(
    text: &str,
    font_size: f32,
    typography: &TextTypography,
    font_asset: Option<&FontAsset>,
) -> Result<TextLayout> {
    validate_inputs(text, font_size, typography)?;
    let fonts = ResolvedFonts::new(font_asset)?;
    let lines = wrapped_lines(text, font_size, typography, &fonts)?;
    let mut shaped_lines = Vec::with_capacity(lines.len());
    let mut natural_width = 1.0_f32;
    let mut natural_height = font_size;
    for range in &lines {
        let raw = shape_range(text, range.clone(), typography, &fonts)?;
        let (advance, metrics) = position_line(&raw, text, font_size, typography.tracking);
        natural_width = natural_width.max(advance);
        natural_height = natural_height.max(metrics);
        shaped_lines.push((raw, advance, metrics));
    }
    let layout_width = typography.box_width.unwrap_or(natural_width).max(1.0);
    let line_height = (natural_height * typography.line_height).ceil().max(1.0);
    let layout_box = Bounds {
        min_x: 0,
        min_y: 0,
        max_x: layout_width.ceil() as i32,
        max_y: (line_height * lines.len().max(1) as f32).ceil() as i32,
    };
    let mut glyphs = Vec::new();
    let mut ink = None::<Bounds>;
    {
        let primary_face = ttf_parser::Face::parse(fonts.bytes(FaceChoice::Primary), 0)
            .context("primary text font is malformed")?;
        let bundled_face = ttf_parser::Face::parse(fonts.bytes(FaceChoice::Bundled), 0)
            .context("bundled Ubuntu fallback is malformed")?;
        for (line_index, (raw, advance, _)) in shaped_lines.iter().enumerate() {
            let alignment = match typography.alignment {
                TextAlignment::Left => 0.0,
                TextAlignment::Center => (layout_width - advance) * 0.5,
                TextAlignment::Right => layout_width - advance,
            }
            .max(0.0);
            let ascent = raw
                .iter()
                .map(|glyph| f32::from(glyph.ascender) * font_size / f32::from(glyph.units_per_em))
                .fold(font_size * 0.8, f32::max);
            let baseline = line_index as f32 * line_height + ascent;
            let mut cursor = alignment;
            for (index, glyph) in raw.iter().enumerate() {
                let scale = font_size / f32::from(glyph.units_per_em);
                let pen_x = cursor;
                let face = match glyph.face {
                    FaceChoice::Primary => &primary_face,
                    FaceChoice::Bundled => &bundled_face,
                };
                if let Some(bitmap) = glyph_pixel_bounds(
                    face,
                    glyph.glyph_id,
                    font_size,
                    pen_x,
                    baseline,
                    glyph.x_offset,
                    glyph.y_offset,
                ) {
                    let bounds = Bounds {
                        min_x: bitmap.left,
                        min_y: bitmap.top,
                        max_x: bitmap.left + bitmap.width as i32,
                        max_y: bitmap.top + bitmap.height as i32,
                    };
                    ink = Some(ink.map_or(bounds, |current| current.union(bounds)));
                }
                glyphs.push(PositionedGlyph {
                    face: glyph.face,
                    glyph_id: glyph.glyph_id,
                    pen_x,
                    baseline,
                    x_offset: glyph.x_offset,
                    y_offset: glyph.y_offset,
                });
                cursor += glyph.x_advance as f32 * scale;
                cursor += tracking_after(raw, index, text, typography.tracking);
            }
        }
    }
    let logical = layout_box;
    let base_visual = ink.unwrap_or(logical);
    let outline = typography.effects.outline_width.ceil() as i32;
    let shadow_x = typography.effects.shadow_offset_x.round() as i32;
    let shadow_y = typography.effects.shadow_offset_y.round() as i32;
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
        fonts,
        font_size,
        glyphs,
        output_min_x: output.min_x,
        output_min_y: output.min_y,
        width,
        height,
        visual,
        layout_box,
    })
}

fn validate_inputs(text: &str, font_size: f32, typography: &TextTypography) -> Result<()> {
    if !font_size.is_finite() || font_size <= 0.0 {
        bail!("text font size must be a positive finite number");
    }
    validate_grapheme_boundaries(text)?;
    if text.len() > spectrum_fonts::MAX_SHAPE_TEXT_BYTES
        || !typography.line_height.is_finite()
        || typography.line_height <= 0.0
        || !typography.tracking.is_finite()
        || typography
            .box_width
            .is_some_and(|width| !width.is_finite() || width <= 0.0)
    {
        bail!("text typography contains invalid shaped layout values");
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
    Ok(())
}

fn wrapped_lines(
    text: &str,
    font_size: f32,
    typography: &TextTypography,
    fonts: &ResolvedFonts,
) -> Result<Vec<Range<usize>>> {
    let mut output = Vec::new();
    let mut start = 0;
    for paragraph_with_break in text.split_inclusive('\n') {
        let paragraph_end = start + paragraph_with_break.trim_end_matches('\n').len();
        wrap_paragraph(
            text,
            start..paragraph_end,
            font_size,
            typography,
            fonts,
            &mut output,
        )?;
        start += paragraph_with_break.len();
    }
    if start < text.len() {
        wrap_paragraph(
            text,
            start..text.len(),
            font_size,
            typography,
            fonts,
            &mut output,
        )?;
    } else if text.ends_with('\n') || text.is_empty() {
        output.push(text.len()..text.len());
    }
    if output.is_empty() {
        output.push(0..0);
    }
    Ok(output)
}

fn wrap_paragraph(
    text: &str,
    paragraph: Range<usize>,
    font_size: f32,
    typography: &TextTypography,
    fonts: &ResolvedFonts,
    output: &mut Vec<Range<usize>>,
) -> Result<()> {
    let Some(limit) = typography.box_width else {
        output.push(paragraph);
        return Ok(());
    };
    if paragraph.is_empty() {
        output.push(paragraph);
        return Ok(());
    }
    let opportunities = linebreaks(&text[paragraph.clone()]).collect::<Vec<_>>();
    if opportunities.len() > MAX_BREAK_OPPORTUNITIES {
        bail!("shaped text exceeds the line-break resource limit");
    }
    let mut line_start = paragraph.start;
    let mut last_fitting = None;
    for (relative_end, opportunity) in opportunities {
        let end = paragraph.start + relative_end;
        let width = measure_range(text, line_start..end, font_size, typography, fonts)?;
        if width <= limit || line_start == end {
            last_fitting = Some(end);
        } else {
            let split = last_fitting
                .filter(|split| *split > line_start)
                .unwrap_or(end);
            output.push(line_start..split);
            line_start = split;
            last_fitting = (split < end).then_some(end);
        }
        if opportunity == BreakOpportunity::Mandatory && line_start < end {
            output.push(line_start..end);
            line_start = end;
            last_fitting = None;
        }
    }
    if line_start < paragraph.end {
        output.push(line_start..paragraph.end);
    }
    Ok(())
}

fn measure_range(
    text: &str,
    range: Range<usize>,
    font_size: f32,
    typography: &TextTypography,
    fonts: &ResolvedFonts,
) -> Result<f32> {
    let glyphs = shape_range(text, range, typography, fonts)?;
    Ok(position_line(&glyphs, text, font_size, typography.tracking).0)
}

fn position_line(glyphs: &[RawGlyph], text: &str, font_size: f32, tracking: f32) -> (f32, f32) {
    let mut advance = 0.0;
    let mut height = font_size;
    for (index, glyph) in glyphs.iter().enumerate() {
        let scale = font_size / f32::from(glyph.units_per_em);
        advance += glyph.x_advance as f32 * scale;
        advance += tracking_after(glyphs, index, text, tracking);
        height = height.max(f32::from(glyph.ascender - glyph.descender + glyph.line_gap) * scale);
    }
    (advance.max(0.0), height.max(1.0))
}

fn tracking_after(glyphs: &[RawGlyph], index: usize, text: &str, tracking: f32) -> f32 {
    let glyph = &glyphs[index];
    if glyphs
        .get(index + 1)
        .is_some_and(|next| next.cluster == glyph.cluster && next.face == glyph.face)
    {
        return 0.0;
    }
    let cluster = glyph.cluster as usize..glyph.cluster_end as usize;
    let boundaries = text[cluster].graphemes(true).count();
    let following_cluster = index + 1 < glyphs.len();
    tracking * (boundaries.saturating_sub(usize::from(!following_cluster))) as f32
}

fn shape_range(
    text: &str,
    range: Range<usize>,
    typography: &TextTypography,
    fonts: &ResolvedFonts,
) -> Result<Vec<RawGlyph>> {
    if range.is_empty() {
        return Ok(Vec::new());
    }
    let bidi = BidiInfo::new(text, None);
    let paragraph = bidi
        .paragraphs
        .iter()
        .find(|paragraph| paragraph.range.start <= range.start && paragraph.range.end >= range.end)
        .context("shaped line is outside a resolved bidi paragraph")?;
    let (_, visual_runs) = bidi.visual_runs(paragraph, range.clone());
    let mut output = Vec::new();
    for visual_run in visual_runs {
        let rtl = bidi.levels[visual_run.start].is_rtl();
        let mut groups = resolved_groups(text, visual_run, fonts)?;
        if rtl {
            groups.reverse();
        }
        for group in groups {
            let direction = if rtl {
                TextDirection::RightToLeft
            } else {
                TextDirection::LeftToRight
            };
            let script = Script::from_iso15924(group.script.as_iso15924_tag().to_be_bytes())?;
            let request = ShapeRequest::new(text)
                .item_range(group.range)
                .direction(direction)
                .script(script)
                .language(typography.shaping.resolved_language());
            let shaped = HarfBuzzShaper::new(fonts.bytes(group.face), 0)?.shape(&request)?;
            output.extend(shaped.glyphs().iter().map(|glyph| RawGlyph {
                face: group.face,
                glyph_id: glyph.glyph_id,
                cluster: glyph.cluster,
                cluster_end: glyph.cluster_end,
                units_per_em: shaped.units_per_em(),
                x_advance: glyph.x_advance,
                x_offset: glyph.x_offset,
                y_offset: glyph.y_offset,
                ascender: shaped.ascender(),
                descender: shaped.descender(),
                line_gap: shaped.line_gap(),
            }));
        }
    }
    Ok(output)
}

#[derive(Clone)]
struct ResolvedGroup {
    range: Range<usize>,
    face: FaceChoice,
    script: UnicodeScript,
}

fn resolved_groups(
    text: &str,
    range: Range<usize>,
    fonts: &ResolvedFonts,
) -> Result<Vec<ResolvedGroup>> {
    let (primary, bundled) = fonts.faces()?;
    let graphemes = text[range.clone()]
        .grapheme_indices(true)
        .map(|(relative, grapheme)| {
            let start = range.start + relative;
            let end = start + grapheme.len();
            let script = grapheme
                .chars()
                .map(|character| character.script())
                .find(|script| !matches!(script, UnicodeScript::Common | UnicodeScript::Inherited))
                .unwrap_or(UnicodeScript::Common);
            Ok((
                start..end,
                fonts.choose_grapheme(&primary, &bundled, grapheme),
                script,
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    let scripts = graphemes
        .iter()
        .map(|(_, _, script)| *script)
        .collect::<Vec<_>>();
    let resolved_scripts = scripts
        .iter()
        .enumerate()
        .map(|(index, script)| {
            if !matches!(script, UnicodeScript::Common | UnicodeScript::Inherited) {
                return *script;
            }
            scripts[..index]
                .iter()
                .rev()
                .copied()
                .find(|candidate| {
                    !matches!(candidate, UnicodeScript::Common | UnicodeScript::Inherited)
                })
                .or_else(|| {
                    scripts[index + 1..].iter().copied().find(|candidate| {
                        !matches!(candidate, UnicodeScript::Common | UnicodeScript::Inherited)
                    })
                })
                .unwrap_or(UnicodeScript::Latin)
        })
        .collect::<Vec<_>>();
    let mut groups = Vec::<ResolvedGroup>::new();
    for ((range, face, _), script) in graphemes.into_iter().zip(resolved_scripts) {
        if let Some(last) = groups.last_mut()
            && last.face == face
            && last.script == script
            && last.range.end == range.start
        {
            last.range.end = range.end;
        } else {
            groups.push(ResolvedGroup {
                range,
                face,
                script,
            });
        }
    }
    Ok(groups)
}

fn render_layout_region(
    layout: &TextLayout,
    color: [u8; 4],
    typography: &TextTypography,
    region: RenderRegion,
) -> Result<RgbaImage> {
    let effects = &typography.effects;
    let radius = if effects.outline_color[3] > 0 {
        effects.outline_width.ceil() as u32
    } else {
        0
    };
    let shadow_padding = if effects.shadow_color[3] > 0 {
        effects
            .shadow_offset_x
            .abs()
            .max(effects.shadow_offset_y.abs())
            .ceil() as u32
    } else {
        0
    };
    let staging = expanded_region(
        region,
        layout.width,
        layout.height,
        radius.saturating_add(shadow_padding),
    );
    if region_pixel_count(region) > MAX_LAYOUT_PIXELS
        || region_pixel_count(staging) > MAX_LAYOUT_PIXELS
    {
        bail!("shaped text exceeds the bounded rendering surface budget");
    }
    let mut base = vec![0_u8; region_pixel_count(staging) as usize];
    let primary_face = ttf_parser::Face::parse(layout.fonts.bytes(FaceChoice::Primary), 0)
        .context("primary text font is malformed")?;
    let bundled_face = ttf_parser::Face::parse(layout.fonts.bytes(FaceChoice::Bundled), 0)
        .context("bundled Ubuntu fallback is malformed")?;
    for glyph in &layout.glyphs {
        let face = match glyph.face {
            FaceChoice::Primary => &primary_face,
            FaceChoice::Bundled => &bundled_face,
        };
        let Some(bounds) = glyph_pixel_bounds(
            face,
            glyph.glyph_id,
            layout.font_size,
            glyph.pen_x,
            glyph.baseline,
            glyph.x_offset,
            glyph.y_offset,
        ) else {
            continue;
        };
        if !bitmap_intersects_region(
            layout,
            bounds.left,
            bounds.top,
            bounds.width,
            bounds.height,
            staging,
        ) {
            continue;
        }
        if let Some(bitmap) = rasterize_glyph(
            layout.fonts.bytes(glyph.face),
            glyph.glyph_id,
            layout.font_size,
            glyph.pen_x,
            glyph.baseline,
            glyph.x_offset,
            glyph.y_offset,
        )? {
            composite_bitmap_alpha(layout, staging, &mut base, &bitmap);
        }
    }
    let mut output = RgbaImage::new(region.width, region.height);
    if effects.shadow_color[3] > 0 {
        paint_staged_alpha(
            &mut output,
            &base,
            staging,
            region,
            effects.shadow_color,
            effects.shadow_offset_x.round() as i32,
            effects.shadow_offset_y.round() as i32,
        );
    }
    if radius > 0 && effects.outline_color[3] > 0 {
        let outline = dilate_alpha(&base, staging.width, staging.height, radius);
        paint_staged_alpha(
            &mut output,
            &outline,
            staging,
            region,
            effects.outline_color,
            0,
            0,
        );
    }
    paint_staged_alpha(&mut output, &base, staging, region, color, 0, 0);
    Ok(output)
}

fn region_pixel_count(region: RenderRegion) -> u64 {
    u64::from(region.width) * u64::from(region.height)
}

fn expanded_region(region: RenderRegion, width: u32, height: u32, padding: u32) -> RenderRegion {
    let x = region.x.saturating_sub(padding);
    let y = region.y.saturating_sub(padding);
    let right = region
        .x
        .saturating_add(region.width)
        .saturating_add(padding)
        .min(width);
    let bottom = region
        .y
        .saturating_add(region.height)
        .saturating_add(padding)
        .min(height);
    RenderRegion {
        x,
        y,
        width: right - x,
        height: bottom - y,
    }
}

fn bitmap_intersects_region(
    layout: &TextLayout,
    left: i32,
    top: i32,
    width: u32,
    height: u32,
    region: RenderRegion,
) -> bool {
    let left = i64::from(left - layout.output_min_x);
    let top = i64::from(top - layout.output_min_y);
    left + i64::from(width) > i64::from(region.x)
        && top + i64::from(height) > i64::from(region.y)
        && left < i64::from(region.x + region.width)
        && top < i64::from(region.y + region.height)
}

fn composite_bitmap_alpha(
    layout: &TextLayout,
    staging: RenderRegion,
    target: &mut [u8],
    bitmap: &GlyphBitmap,
) {
    let left = bitmap.left - layout.output_min_x - staging.x as i32;
    let top = bitmap.top - layout.output_min_y - staging.y as i32;
    for row in 0..bitmap.height {
        for column in 0..bitmap.width {
            let x = left + column as i32;
            let y = top + row as i32;
            if x >= 0 && y >= 0 && x < staging.width as i32 && y < staging.height as i32 {
                let target_index = y as usize * staging.width as usize + x as usize;
                let source_index = row as usize * bitmap.width as usize + column as usize;
                target[target_index] = target[target_index].max(bitmap.alpha[source_index]);
            }
        }
    }
}

fn paint_staged_alpha(
    output: &mut RgbaImage,
    alpha: &[u8],
    staging: RenderRegion,
    output_region: RenderRegion,
    color: [u8; 4],
    shift_x: i32,
    shift_y: i32,
) {
    for y in 0..output.height() {
        for x in 0..output.width() {
            let source_x =
                output_region.x as i64 + i64::from(x) - i64::from(shift_x) - i64::from(staging.x);
            let source_y =
                output_region.y as i64 + i64::from(y) - i64::from(shift_y) - i64::from(staging.y);
            if source_x < 0
                || source_y < 0
                || source_x >= i64::from(staging.width)
                || source_y >= i64::from(staging.height)
            {
                continue;
            }
            let coverage = alpha[source_y as usize * staging.width as usize + source_x as usize];
            let alpha = u16::from(coverage) * u16::from(color[3]) / 255;
            composite_over(output, x, y, [color[0], color[1], color[2], alpha as u8]);
        }
    }
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
        output[channel] = ((f32::from(source[channel]) * source_alpha
            + f32::from(destination[channel]) * destination_alpha * (1.0 - source_alpha))
            / output_alpha)
            .round() as u8;
    }
    output[3] = (output_alpha * 255.0).round() as u8;
    image.put_pixel(x, y, Rgba(output));
}

#[cfg(test)]
#[path = "shaped_tests.rs"]
mod tests;
