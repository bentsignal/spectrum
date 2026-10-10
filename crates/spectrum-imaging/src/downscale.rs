//! Shrinking large photos for previews, fast: a whole-number box average on
//! every core, then a small Triangle resize for the last step. Exports keep
//! the exact single-pass resize; previews only need to look the same.
use image::{DynamicImage, RgbImage, RgbaImage};
use rayon::prelude::*;

/// `image` fitted within `max_size` on both sides, at the size
/// `DynamicImage::resize` gives.
pub fn downscale(image: &DynamicImage, max_size: u32) -> DynamicImage {
    let (width, height) = (image.width(), image.height());
    if max_size == 0 || (width <= max_size && height <= max_size) {
        return image.clone();
    }
    let (target_width, target_height) = fitted(width, height, max_size);
    let factor = (width / target_width).min(height / target_height).max(1);
    let target = (target_width, target_height);
    if image.color().has_alpha() {
        let rgba = image.to_rgba8();
        let pixels = shrink(rgba.as_raw(), (width, height), 4, factor, target);
        DynamicImage::ImageRgba8(
            RgbaImage::from_raw(target_width, target_height, pixels).expect("whole pixels"),
        )
    } else {
        let converted;
        let rgb = match image {
            DynamicImage::ImageRgb8(rgb) => rgb,
            other => {
                converted = other.to_rgb8();
                &converted
            }
        };
        let pixels = shrink(rgb.as_raw(), (width, height), 3, factor, target);
        DynamicImage::ImageRgb8(
            RgbImage::from_raw(target_width, target_height, pixels).expect("whole pixels"),
        )
    }
}

/// A box average by `factor`, then a Triangle resample to `target`.
fn shrink(
    raw: &[u8],
    size: (u32, u32),
    channels: usize,
    factor: u32,
    target: (u32, u32),
) -> Vec<u8> {
    if factor < 2 {
        return resample(raw, size, channels, target);
    }
    let (reduced, width, height) = box_reduce(raw, size.0, size.1, channels, factor);
    if (width, height) == target {
        reduced
    } else {
        resample(&reduced, (width, height), channels, target)
    }
}

/// Triangle-filter weights for each output position along one axis.
fn taps(from: u32, to: u32) -> Vec<(usize, Vec<f32>)> {
    let ratio = from as f32 / to as f32;
    let support = ratio.max(1.0);
    (0..to)
        .map(|out| {
            let center = (out as f32 + 0.5) * ratio;
            let first = ((center - support).floor().max(0.0)) as usize;
            let last = ((center + support).ceil() as usize).min(from as usize);
            let mut weights: Vec<f32> = (first..last)
                .map(|index| (1.0 - ((index as f32 + 0.5 - center) / support).abs()).max(0.0))
                .collect();
            let total: f32 = weights.iter().sum();
            if total > 0.0 {
                weights.iter_mut().for_each(|weight| *weight /= total);
            }
            (first, weights)
        })
        .collect()
}

/// A separable Triangle resample, rows in parallel.
fn resample(raw: &[u8], size: (u32, u32), channels: usize, target: (u32, u32)) -> Vec<u8> {
    let (columns, rows) = (taps(size.0, target.0), taps(size.1, target.1));
    let row = size.0 as usize * channels;
    let out_row = target.0 as usize * channels;
    // Horizontal pass: every source row at the target width.
    let mut wide = vec![0f32; out_row * size.1 as usize];
    wide.par_chunks_mut(out_row)
        .zip(raw.par_chunks(row))
        .for_each(|(out, source)| {
            for (x, (first, weights)) in columns.iter().enumerate() {
                for (offset, weight) in weights.iter().enumerate() {
                    let pixel = &source[(first + offset) * channels..][..channels];
                    for channel in 0..channels {
                        out[x * channels + channel] += f32::from(pixel[channel]) * weight;
                    }
                }
            }
        });
    // Vertical pass.
    let mut output = vec![0u8; out_row * target.1 as usize];
    output
        .par_chunks_mut(out_row)
        .zip(rows.par_iter())
        .for_each(|(out, (first, weights))| {
            let mut sums = vec![0f32; out_row];
            for (offset, weight) in weights.iter().enumerate() {
                let source = &wide[(first + offset) * out_row..][..out_row];
                for (sum, value) in sums.iter_mut().zip(source) {
                    *sum += value * weight;
                }
            }
            for (out, sum) in out.iter_mut().zip(sums) {
                *out = (sum + 0.5).clamp(0.0, 255.0) as u8;
            }
        });
    output
}

/// The size `DynamicImage::resize` gives `width` x `height` within `max_size`.
fn fitted(width: u32, height: u32, max_size: u32) -> (u32, u32) {
    let ratio = (max_size as f64 / width as f64).min(max_size as f64 / height as f64);
    (
        ((width as f64 * ratio).round() as u32).max(1),
        ((height as f64 * ratio).round() as u32).max(1),
    )
}

/// Averages each `factor` x `factor` block into one pixel, rows in
/// parallel. Edge pixels that do not fill a block are left out.
fn box_reduce(
    raw: &[u8],
    width: u32,
    height: u32,
    channels: usize,
    factor: u32,
) -> (Vec<u8>, u32, u32) {
    let (out_width, out_height) = (width / factor, height / factor);
    let (factor, row) = (factor as usize, width as usize * channels);
    let count = (factor * factor) as u32;
    let mut output = vec![0u8; out_width as usize * out_height as usize * channels];
    output
        .par_chunks_mut(out_width as usize * channels)
        .enumerate()
        .for_each(|(y, out_row)| {
            let mut sums = vec![0u32; out_row.len()];
            for source_row in raw[y * factor * row..].chunks_exact(row).take(factor) {
                for (x, sum) in sums.chunks_exact_mut(channels).enumerate() {
                    let block = &source_row[x * factor * channels..(x + 1) * factor * channels];
                    for pixel in block.chunks_exact(channels) {
                        for (sum, value) in sum.iter_mut().zip(pixel) {
                            *sum += u32::from(*value);
                        }
                    }
                }
            }
            for (out, sum) in out_row.iter_mut().zip(sums) {
                *out = ((sum + count / 2) / count) as u8;
            }
        });
    (output, out_width, out_height)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn previews_match_the_size_and_look_of_a_full_resize() {
        let image = DynamicImage::ImageRgb8(RgbImage::from_fn(1203, 801, |x, y| {
            image::Rgb([(x / 5) as u8, (y / 4) as u8, ((x + y) / 9) as u8])
        }));
        let near = downscale(&image, 900);
        let near_exact = image.resize(900, 900, image::imageops::FilterType::Triangle);
        assert_eq!(
            (near.width(), near.height()),
            (near_exact.width(), near_exact.height())
        );
        let fast = downscale(&image, 256);
        let exact = image.resize(256, 256, image::imageops::FilterType::Triangle);
        assert_eq!(
            (fast.width(), fast.height()),
            (exact.width(), exact.height())
        );
        let (fast, exact) = (fast.to_rgb8(), exact.to_rgb8());
        let worst = fast
            .as_raw()
            .iter()
            .zip(exact.as_raw())
            .map(|(a, b)| a.abs_diff(*b))
            .max()
            .unwrap_or(0);
        assert!(worst <= 3, "differs by up to {worst}");
    }
}
