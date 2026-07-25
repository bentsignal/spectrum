use std::collections::BTreeMap;

use anyhow::{Result, bail};
use image::{Rgba, RgbaImage};
use spectrum_imaging::PixelRegion;

use crate::{
    BrushMode, BrushProgram, BrushStroke, MAX_PAINT_REGION_PIXELS, PixelMask, RasterSourceResolver,
    sampled_source::sampled_source_region,
};

#[cfg(test)]
pub(crate) fn render_paint_region(
    program: &BrushProgram,
    pixel_mask: Option<&PixelMask>,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
) -> Result<RgbaImage> {
    render_paint_region_with_sources(program, pixel_mask, x, y, width, height, None)
}

pub(crate) fn render_paint_region_with_sources(
    program: &BrushProgram,
    pixel_mask: Option<&PixelMask>,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    raster_sources: Option<&dyn RasterSourceResolver>,
) -> Result<RgbaImage> {
    let pixels = u64::from(width) * u64::from(height);
    if width == 0 || height == 0 || pixels > MAX_PAINT_REGION_PIXELS {
        bail!("Paint render region exceeds the bounded 4096-square limit");
    }
    if x.checked_add(width)
        .is_none_or(|right| right > program.width)
        || y.checked_add(height)
            .is_none_or(|bottom| bottom > program.height)
    {
        bail!("Paint render region exceeds the Paint viewport");
    }
    let mut output = RgbaImage::new(width, height);
    for stroke in program.strokes.iter() {
        let requested = (x, y, width, height);
        let sampled = stage_sampled_source(stroke, requested, raster_sources)?;
        let mut tiles = BTreeMap::<(u32, u32), Vec<u8>>::new();
        for_each_dab(stroke, |dab| {
            for_dab_tiles(dab, stroke.style.size, requested, |key, tile| {
                let coverage = tiles
                    .entry(key)
                    .or_insert_with(|| vec![0; (tile.2 * tile.3) as usize]);
                accumulate_dab(coverage, tile, stroke, dab);
            });
        });
        for (key, coverage) in tiles {
            let tile = paint_tile_region(key, requested);
            for source_y in tile.1..tile.1 + tile.3 {
                for source_x in tile.0..tile.0 + tile.2 {
                    let index = ((source_y - tile.1) * tile.2 + (source_x - tile.0)) as usize;
                    let mut alpha = u32::from(coverage[index]);
                    if let Some(clip) = &stroke.clip {
                        alpha = (alpha * u32::from(clip.alpha_at(source_x, source_y)) + 127) / 255;
                    }
                    alpha = (alpha * (stroke.style.opacity * 255.0).round() as u32 + 127) / 255;
                    let sampled_pixel = sampled
                        .as_ref()
                        .and_then(|sampled| sampled.pixel(source_x, source_y));
                    alpha = match stroke.style.mode {
                        BrushMode::Paint => (alpha * u32::from(stroke.style.color[3]) + 127) / 255,
                        BrushMode::CloneStamp => {
                            (alpha * u32::from(sampled_pixel.map_or(0, |pixel| pixel[3])) + 127)
                                / 255
                        }
                        BrushMode::Erase => alpha,
                    };
                    if alpha == 0 {
                        continue;
                    }
                    let destination = output.get_pixel_mut(source_x - x, source_y - y);
                    match stroke.style.mode {
                        BrushMode::Paint => {
                            source_over(destination, stroke.style.color, alpha as u8)
                        }
                        BrushMode::Erase => destination_out(destination, alpha as u8),
                        BrushMode::CloneStamp => source_over(
                            destination,
                            sampled_pixel.expect("nonzero sampled alpha has a source pixel"),
                            alpha as u8,
                        ),
                    }
                }
            }
        }
    }
    if let Some(mask) = pixel_mask {
        if (mask.width, mask.height) != (program.width, program.height) {
            bail!("Paint pixel mask dimensions do not match its viewport");
        }
        for local_y in 0..height {
            for local_x in 0..width {
                let mask_alpha =
                    u16::from(mask.alpha[((y + local_y) * mask.width + x + local_x) as usize]);
                let pixel = output.get_pixel_mut(local_x, local_y);
                pixel[3] = ((u16::from(pixel[3]) * mask_alpha + 127) / 255) as u8;
                if pixel[3] == 0 {
                    *pixel = Rgba([0; 4]);
                }
            }
        }
    }
    Ok(output)
}

struct StagedSampledSource {
    destination_shift: [i64; 2],
    region: PixelRegion,
    image: RgbaImage,
}

impl StagedSampledSource {
    fn pixel(&self, destination_x: u32, destination_y: u32) -> Option<[u8; 4]> {
        let source_x = i64::from(destination_x) + self.destination_shift[0];
        let source_y = i64::from(destination_y) + self.destination_shift[1];
        if source_x < i64::from(self.region.x)
            || source_y < i64::from(self.region.y)
            || source_x >= i64::from(self.region.x + self.region.width)
            || source_y >= i64::from(self.region.y + self.region.height)
        {
            return None;
        }
        Some(
            self.image
                .get_pixel(
                    source_x as u32 - self.region.x,
                    source_y as u32 - self.region.y,
                )
                .0,
        )
    }
}

fn stage_sampled_source(
    stroke: &BrushStroke,
    requested: (u32, u32, u32, u32),
    raster_sources: Option<&dyn RasterSourceResolver>,
) -> Result<Option<StagedSampledSource>> {
    if stroke.style.mode != BrushMode::CloneStamp {
        return Ok(None);
    }
    let source = stroke
        .sampled_source()
        .ok_or_else(|| anyhow::anyhow!("Clone Stamp stroke has no resolved sampled source"))?;
    let first = stroke.samples[0];
    let destination_shift = [
        (source.anchor_local[0] - first.x + 0.5).floor() as i64,
        (source.anchor_local[1] - first.y + 0.5).floor() as i64,
    ];
    let source_left = i64::from(requested.0) + destination_shift[0];
    let source_top = i64::from(requested.1) + destination_shift[1];
    let source_right = i64::from(requested.0 + requested.2) + destination_shift[0];
    let source_bottom = i64::from(requested.1 + requested.3) + destination_shift[1];
    let left = source_left.clamp(0, i64::from(source.width)) as u32;
    let top = source_top.clamp(0, i64::from(source.height)) as u32;
    let right = source_right.clamp(0, i64::from(source.width)) as u32;
    let bottom = source_bottom.clamp(0, i64::from(source.height)) as u32;
    if right <= left || bottom <= top {
        return Ok(Some(StagedSampledSource {
            destination_shift,
            region: PixelRegion {
                x: 0,
                y: 0,
                width: 0,
                height: 0,
            },
            image: RgbaImage::new(0, 0),
        }));
    }
    let region = PixelRegion {
        x: left,
        y: top,
        width: right - left,
        height: bottom - top,
    };
    let image = sampled_source_region(source, region, raster_sources)?;
    Ok(Some(StagedSampledSource {
        destination_shift,
        region,
        image,
    }))
}

const PAINT_TILE_SIZE: u32 = 64;

fn for_dab_tiles(
    dab: Dab,
    brush_size: f32,
    requested: (u32, u32, u32, u32),
    mut visit: impl FnMut((u32, u32), (u32, u32, u32, u32)),
) {
    let radius = brush_size * dab.pressure * 0.5 + 0.5;
    if radius <= 0.5 {
        return;
    }
    let left = (dab.x - radius).floor().max(requested.0 as f32) as u32;
    let top = (dab.y - radius).floor().max(requested.1 as f32) as u32;
    let right = (dab.x + radius)
        .ceil()
        .min((requested.0 + requested.2) as f32) as u32;
    let bottom = (dab.y + radius)
        .ceil()
        .min((requested.1 + requested.3) as f32) as u32;
    if right <= left || bottom <= top {
        return;
    }
    for tile_y in top / PAINT_TILE_SIZE..=(bottom - 1) / PAINT_TILE_SIZE {
        for tile_x in left / PAINT_TILE_SIZE..=(right - 1) / PAINT_TILE_SIZE {
            let key = (tile_x, tile_y);
            visit(key, paint_tile_region(key, requested));
        }
    }
}

fn paint_tile_region(key: (u32, u32), requested: (u32, u32, u32, u32)) -> (u32, u32, u32, u32) {
    let left = (key.0 * PAINT_TILE_SIZE).max(requested.0);
    let top = (key.1 * PAINT_TILE_SIZE).max(requested.1);
    let right = ((key.0 + 1) * PAINT_TILE_SIZE).min(requested.0 + requested.2);
    let bottom = ((key.1 + 1) * PAINT_TILE_SIZE).min(requested.1 + requested.3);
    (left, top, right - left, bottom - top)
}

#[derive(Clone, Copy)]
struct Dab {
    x: f32,
    y: f32,
    pressure: f32,
}

fn for_each_dab(stroke: &BrushStroke, mut visit: impl FnMut(Dab)) {
    let first = stroke.samples[0];
    visit(Dab {
        x: first.x,
        y: first.y,
        pressure: first.pressure,
    });
    let interval = stroke.interval();
    let mut distance_to_next = interval;
    let mut last_emitted = (first.x, first.y);
    for pair in stroke.samples.windows(2) {
        let start = pair[0];
        let end = pair[1];
        let dx = end.x - start.x;
        let dy = end.y - start.y;
        let length = dx.hypot(dy);
        if length <= f32::EPSILON {
            continue;
        }
        let mut traveled = distance_to_next;
        while traveled <= length {
            let t = traveled / length;
            let dab = Dab {
                x: start.x + dx * t,
                y: start.y + dy * t,
                pressure: start.pressure + (end.pressure - start.pressure) * t,
            };
            visit(dab);
            last_emitted = (dab.x, dab.y);
            traveled += interval;
        }
        distance_to_next = traveled - length;
    }
    let last = *stroke.samples.last().expect("validated stroke is nonempty");
    if (last.x - last_emitted.0).hypot(last.y - last_emitted.1) > 0.0001 {
        visit(Dab {
            x: last.x,
            y: last.y,
            pressure: last.pressure,
        });
    }
}

fn accumulate_dab(
    coverage: &mut [u8],
    region: (u32, u32, u32, u32),
    stroke: &BrushStroke,
    dab: Dab,
) {
    let radius = stroke.style.size * dab.pressure * 0.5;
    if radius <= 0.0 {
        return;
    }
    let extent = radius + 0.5;
    let left = (dab.x - extent).floor().max(region.0 as f32) as u32;
    let top = (dab.y - extent).floor().max(region.1 as f32) as u32;
    let right = (dab.x + extent).ceil().min((region.0 + region.2) as f32) as u32;
    let bottom = (dab.y + extent).ceil().min((region.1 + region.3) as f32) as u32;
    let hard_radius = radius * stroke.style.hardness;
    for y in top..bottom {
        for x in left..right {
            let distance =
                ((x as f32 + 0.5 - dab.x).powi(2) + (y as f32 + 0.5 - dab.y).powi(2)).sqrt();
            let edge = radius + 0.5;
            let radial = if distance <= hard_radius {
                1.0
            } else {
                ((edge - distance) / (edge - hard_radius).max(0.0001)).clamp(0.0, 1.0)
            };
            let value = (radial * dab.pressure * 255.0).round() as u8;
            let index = ((y - region.1) * region.2 + (x - region.0)) as usize;
            coverage[index] = coverage[index].max(value);
        }
    }
}

fn source_over(destination: &mut Rgba<u8>, color: [u8; 4], source_alpha: u8) {
    let source_alpha = u32::from(source_alpha);
    let destination_alpha = u32::from(destination[3]);
    let retained = (destination_alpha * (255 - source_alpha) + 127) / 255;
    let output_alpha = source_alpha + retained;
    if output_alpha == 0 {
        *destination = Rgba([0; 4]);
        return;
    }
    for channel in 0..3 {
        destination[channel] = ((u32::from(color[channel]) * source_alpha
            + u32::from(destination[channel]) * retained
            + output_alpha / 2)
            / output_alpha) as u8;
    }
    destination[3] = output_alpha.min(255) as u8;
}

fn destination_out(destination: &mut Rgba<u8>, source_alpha: u8) {
    let alpha = (u32::from(destination[3]) * (255 - u32::from(source_alpha)) + 127) / 255;
    destination[3] = alpha as u8;
    if alpha == 0 {
        *destination = Rgba([0; 4]);
    }
}
