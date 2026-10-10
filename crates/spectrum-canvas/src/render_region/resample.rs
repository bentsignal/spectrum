//! Resizing a staged source to the layer's scaled size once, before it is
//! sampled. `sample_triangle_resize` weighs every source pixel under a
//! scaled pixel's triangle window, per output pixel; done here as a vertical
//! pass per scaled row and then a horizontal pass, with the same weights in
//! the same order, it gives the same bytes at a fraction of the work, and
//! every later sample becomes a lookup.
use image::RgbaImage;
use rayon::prelude::*;

use super::{
    SamplingGeometry,
    source::{
        SUMS, SampleSource, SourceRegion, accumulate_premultiplied, source_sample_bounds,
        triangle_weight_parts, unpremultiplied,
    },
};

/// The scaled coordinates along one axis whose triangle windows lie inside
/// the staged source range `[start, end)`.
fn covered(source: u32, scaled: u32, start: u32, end: u32) -> Option<(u32, u32)> {
    let inside = |coordinate: u32| {
        let window = source_sample_bounds(source, scaled, coordinate);
        window.start >= start && window.end <= end
    };
    let first = (0..scaled).find(|&c| inside(c))?;
    let last = (first..scaled).take_while(|&c| inside(c)).last()?;
    Some((first, last + 1))
}

/// `source` resized to `geometry`'s scaled size over every scaled pixel its
/// staged region can produce, and the geometry to sample it with (source
/// and scaled sizes equal, so samples are lookups). Sources that are not
/// staged pixels, or are already at scale, stay as they are.
pub(super) fn presample<'a>(
    source: SampleSource<'a>,
    geometry: SamplingGeometry,
) -> (SampleSource<'a>, SamplingGeometry) {
    let SampleSource::Pixels { image, region } = &source else {
        return (source, geometry);
    };
    let (sw, sh) = (geometry.source_width, geometry.source_height);
    let (dw, dh) = (geometry.scaled_width, geometry.scaled_height);
    if (sw, sh) == (dw, dh) {
        return (source, geometry);
    }
    let Some((x0, x1)) = covered(sw, dw, region.x, region.x + region.width) else {
        return (source, geometry);
    };
    let Some((y0, y1)) = covered(sh, dh, region.y, region.y + region.height) else {
        return (source, geometry);
    };
    let (width, height) = (x1 - x0, y1 - y0);
    let staged = region.x..region.x + region.width;
    let mut out = RgbaImage::new(width, height);
    out.par_chunks_mut(width as usize * 4)
        .enumerate()
        .for_each(|(row, line)| {
            let scaled_y = y0 + row as u32;
            let (ys, y_weight) = triangle_weight_parts(sh, dh, scaled_y);
            // Each staged column filtered down this scaled row.
            let columns: Vec<[f32; SUMS]> = staged
                .clone()
                .map(|source_x| {
                    let mut vertical = [0.0_f32; SUMS];
                    for source_y in ys.clone() {
                        let pixel = image.get_pixel(source_x - region.x, source_y - region.y).0;
                        accumulate_premultiplied(&mut vertical, pixel, y_weight(source_y));
                    }
                    vertical
                })
                .collect();
            for (column, slot) in line.chunks_mut(4).enumerate() {
                let scaled_x = x0 + column as u32;
                let (xs, x_weight) = triangle_weight_parts(sw, dw, scaled_x);
                let mut horizontal = [0.0_f32; SUMS];
                for source_x in xs {
                    let vertical = columns[(source_x - region.x) as usize];
                    let weight = x_weight(source_x);
                    for channel in 0..SUMS {
                        horizontal[channel] += vertical[channel] * weight;
                    }
                }
                slot.copy_from_slice(&unpremultiplied(horizontal));
            }
        });
    let resized = SampleSource::Pixels {
        image: out,
        region: SourceRegion {
            x: x0,
            y: y0,
            width,
            height,
        },
    };
    let geometry = SamplingGeometry {
        source_width: dw,
        source_height: dh,
        ..geometry
    };
    (resized, geometry)
}

/// Multiplies a deferred painted alpha into a staged (and maybe resized)
/// source, stretched over the whole of it as the geometry sizes it.
pub(super) fn apply_painted(
    source: &mut SampleSource<'_>,
    painted: Option<&crate::PixelMask>,
    geometry: SamplingGeometry,
) {
    let (Some(painted), SampleSource::Pixels { image, region }) = (painted, source) else {
        return;
    };
    crate::layer_erase::apply_painted_alpha(
        image,
        painted,
        (geometry.source_width, geometry.source_height),
        (region.x, region.y),
    );
}

/// Interactive clients turn this on: renders of the same authenticated
/// raster at the same scale reuse its staged, resized pixels, so dragging,
/// rotating, or restyling an image re-renders without reading it again.
/// Off by default, so exports and benchmarks always read their sources.
static CACHE_ENABLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn set_interactive_source_cache(enabled: bool) {
    CACHE_ENABLED.store(enabled, std::sync::atomic::Ordering::Relaxed);
    if !enabled {
        CACHE.lock().unwrap_or_else(|e| e.into_inner()).clear();
        crate::typography::clear_font_caches();
    }
}

/// Whether interactive clients asked to keep sources between renders.
pub(crate) fn interactive_caches() -> bool {
    CACHE_ENABLED.load(std::sync::atomic::Ordering::Relaxed)
}

/// The most entries and bytes kept. Every raster layer on a canvas keeps
/// two (its reduced copy and that copy at the layer's scale), so the count
/// allows a canvas of many photos; the bytes bound the memory.
const ENTRIES: usize = 512;
const BYTES: usize = 384 << 20;

struct Entry {
    key: String,
    image: std::sync::Arc<RgbaImage>,
    region: SourceRegion,
    source_size: (u32, u32),
}

static CACHE: std::sync::Mutex<Vec<Entry>> = std::sync::Mutex::new(Vec::new());

/// A cached resized source for `key`, as presampled source and geometry.
pub(super) fn cached<'a>(
    key: &str,
    geometry: SamplingGeometry,
) -> Option<(SampleSource<'a>, SamplingGeometry)> {
    if !CACHE_ENABLED.load(std::sync::atomic::Ordering::Relaxed) {
        return None;
    }
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    let index = cache.iter().position(|entry| entry.key == key)?;
    // Most recently used last.
    let entry = cache.remove(index);
    let found = (
        SampleSource::Pixels {
            image: (*entry.image).clone(),
            region: entry.region,
        },
        SamplingGeometry {
            source_width: entry.source_size.0,
            source_height: entry.source_size.1,
            ..geometry
        },
    );
    cache.push(entry);
    Some(found)
}

/// Keeps a presampled source for later renders of the same thing.
pub(super) fn remember(key: String, source: &SampleSource<'_>, geometry: SamplingGeometry) {
    if !CACHE_ENABLED.load(std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    let SampleSource::Pixels { image, region } = source else {
        return;
    };
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    cache.retain(|entry| entry.key != key);
    cache.push(Entry {
        key,
        image: std::sync::Arc::new(image.clone()),
        region: *region,
        source_size: (geometry.source_width, geometry.source_height),
    });
    let bytes = |cache: &Vec<Entry>| cache.iter().map(|e| e.image.as_raw().len()).sum::<usize>();
    while cache.len() > ENTRIES || (cache.len() > 1 && bytes(&cache) > BYTES) {
        cache.remove(0);
    }
}

/// A whole image resized to `width` × `height` with the same weights and
/// alpha weighing as region samples, so full renders and regions agree.
pub(crate) fn resize_whole(image: &RgbaImage, width: u32, height: u32) -> RgbaImage {
    let (sw, sh) = image.dimensions();
    let mut out = RgbaImage::new(width, height);
    out.par_chunks_mut(width as usize * 4)
        .enumerate()
        .for_each(|(row, line)| {
            let (ys, y_weight) = triangle_weight_parts(sh, height, row as u32);
            let columns: Vec<[f32; SUMS]> = (0..sw)
                .map(|source_x| {
                    let mut vertical = [0.0_f32; SUMS];
                    for source_y in ys.clone() {
                        let pixel = image.get_pixel(source_x, source_y).0;
                        accumulate_premultiplied(&mut vertical, pixel, y_weight(source_y));
                    }
                    vertical
                })
                .collect();
            for (column, slot) in line.chunks_mut(4).enumerate() {
                let (xs, x_weight) = triangle_weight_parts(sw, width, column as u32);
                let mut horizontal = [0.0_f32; SUMS];
                for source_x in xs {
                    let vertical = columns[source_x as usize];
                    let weight = x_weight(source_x);
                    for channel in 0..SUMS {
                        horizontal[channel] += vertical[channel] * weight;
                    }
                }
                slot.copy_from_slice(&unpremultiplied(horizontal));
            }
        });
    out
}
