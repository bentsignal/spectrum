//! Large rasters shown at half their size or less, drawn from a reduced copy.
//!
//! Interactive clients only: a canvas showing a 24-megapixel photo scaled
//! to fit would otherwise read and resize every source pixel on every zoom
//! step. The copy (the adjusted, masked source averaged down by a power of
//! two, never below the size it is shown at) is made once in strips within
//! the staging budget and kept with the other staged sources. Exports
//! always read the source exactly.
use image::RgbaImage;
use rayon::prelude::*;

use super::{
    RegionRenderStats, SamplingGeometry,
    resample::{self, interactive_caches},
    source::{
        SUMS, SampleSource, SourceDescriptor, SourceRegion, accumulate_premultiplied,
        unpremultiplied,
    },
};

/// Sources smaller than this are staged directly.
const MIN_PIXELS: u64 = 4_000_000;
/// Source pixels staged at once while making a copy.
const STRIP_PIXELS: u64 = 4 << 20;

/// The source to draw `descriptor` from at `geometry`'s scale, resized from
/// its reduced copy, and the geometry to sample it with, when one applies.
pub(super) fn level<'a>(
    descriptor: &SourceDescriptor<'a>,
    geometry: SamplingGeometry,
    stats: &mut RegionRenderStats,
) -> anyhow::Result<Option<(SampleSource<'a>, SamplingGeometry)>> {
    if !interactive_caches() {
        return Ok(None);
    }
    let (width, height) = (geometry.source_width, geometry.source_height);
    if u64::from(width) * u64::from(height) < MIN_PIXELS {
        return Ok(None);
    }
    let mut shift = 0;
    while shift < 6
        && (width >> (shift + 1)) >= geometry.scaled_width.max(1)
        && (height >> (shift + 1)) >= geometry.scaled_height.max(1)
    {
        shift += 1;
    }
    if shift == 0 {
        return Ok(None);
    }
    let whole = SourceRegion {
        x: 0,
        y: 0,
        width,
        height,
    };
    let Some(identity) = descriptor.cache_key(whole) else {
        return Ok(None);
    };
    let key = format!("{identity}|level{shift}");
    let (level_width, level_height) = ((width >> shift).max(1), (height >> shift).max(1));
    let level_geometry = SamplingGeometry {
        source_width: level_width,
        source_height: level_height,
        ..geometry
    };
    let at_scale = format!("{key}|{}x{}", geometry.scaled_width, geometry.scaled_height);
    if let Some(found) = resample::cached(&at_scale, geometry) {
        return Ok(Some(found));
    }
    let level = match resample::cached(&key, level_geometry) {
        Some((level, _)) => Some(level),
        None => make(descriptor, (width, height), shift, stats)?.inspect(|level| {
            resample::remember(key, level, level_geometry);
        }),
    };
    let Some(level) = level else {
        return Ok(None);
    };
    let (presampled, presampled_geometry) = resample::presample(level, level_geometry);
    resample::remember(at_scale, &presampled, presampled_geometry);
    Ok(Some((presampled, presampled_geometry)))
}

/// The adjusted, masked source averaged down by `2^shift`, staged in strips.
fn make<'a>(
    descriptor: &SourceDescriptor<'a>,
    (width, height): (u32, u32),
    shift: u32,
    stats: &mut RegionRenderStats,
) -> anyhow::Result<Option<SampleSource<'a>>> {
    let (level_width, level_height) = ((width >> shift).max(1), (height >> shift).max(1));
    let factor = 1u32 << shift;
    let mut image = RgbaImage::new(level_width, level_height);
    // Whole level rows per strip, so each strip fills its own rows.
    let rows = ((STRIP_PIXELS / u64::from(width)) as u32 / factor).max(1);
    for first in (0..level_height).step_by(rows as usize) {
        let last = (first + rows).min(level_height);
        let top = first * factor;
        let bottom = if last == level_height {
            height
        } else {
            last * factor
        };
        let strip = SourceRegion {
            x: 0,
            y: top,
            width,
            height: bottom - top,
        };
        let SampleSource::Pixels { image: staged, .. } = descriptor.sample(strip, stats)? else {
            return Ok(None);
        };
        let line = level_width as usize * 4;
        image.as_mut()[first as usize * line..last as usize * line]
            .par_chunks_mut(line)
            .enumerate()
            .for_each(|(offset, out)| {
                let row = first + offset as u32;
                let y0 = row * factor - top;
                let y1 = if row + 1 == level_height {
                    bottom - top
                } else {
                    (row + 1) * factor - top
                };
                for (column, slot) in out.chunks_mut(4).enumerate() {
                    let x0 = column as u32 * factor;
                    let x1 = if column as u32 + 1 == level_width {
                        width
                    } else {
                        x0 + factor
                    };
                    let mut sum = [0.0_f32; SUMS];
                    let weight = 1.0 / ((x1 - x0) * (y1 - y0)) as f32;
                    for y in y0..y1 {
                        for x in x0..x1 {
                            accumulate_premultiplied(&mut sum, staged.get_pixel(x, y).0, weight);
                        }
                    }
                    slot.copy_from_slice(&unpremultiplied(sum));
                }
            });
    }
    Ok(Some(SampleSource::Pixels {
        image,
        region: SourceRegion {
            x: 0,
            y: 0,
            width: level_width,
            height: level_height,
        },
    }))
}
