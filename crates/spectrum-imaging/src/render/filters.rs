//! Whole-image filters, run across every core. Each gives exactly the pixels
//! its single-threaded form gives, so exports and previews never differ.
use image::{DynamicImage, Rgba, RgbaImage};
use rayon::prelude::*;

/// Rows of context each blur strip reads beyond the rows it keeps: more than
/// the kernel reaches for any sigma below [`MAX_STRIP_SIGMA`].
const BLUR_HALO: u32 = 16;
const MAX_STRIP_SIGMA: f32 = 4.0;
/// Images smaller than this blur in one piece.
const STRIP_ROWS: u32 = 64;

/// A Gaussian blur equal to `DynamicImage::blur`, in horizontal strips run in
/// parallel. Every kept row sees the same neighbors it does in the whole
/// image, so the result matches bit for bit.
pub(crate) fn blur(image: &RgbaImage, sigma: f32) -> RgbaImage {
    let (width, height) = image.dimensions();
    let strip = (height / rayon::current_num_threads().max(1) as u32 / 2).max(STRIP_ROWS);
    if sigma > MAX_STRIP_SIGMA || height <= strip || width == 0 {
        return DynamicImage::ImageRgba8(image.clone())
            .blur(sigma)
            .to_rgba8();
    }
    let row_bytes = width as usize * 4;
    let mut output = RgbaImage::new(width, height);
    output
        .as_mut()
        .par_chunks_mut(row_bytes * strip as usize)
        .enumerate()
        .for_each(|(index, rows)| {
            let top = index as u32 * strip;
            let kept = (rows.len() / row_bytes) as u32;
            let from = top.saturating_sub(BLUR_HALO);
            let to = (top + kept + BLUR_HALO).min(height);
            let source = &image.as_raw()[from as usize * row_bytes..to as usize * row_bytes];
            let piece = RgbaImage::from_raw(width, to - from, source.to_vec())
                .expect("strip holds whole rows");
            let blurred = DynamicImage::ImageRgba8(piece).blur(sigma).to_rgba8();
            let start = (top - from) as usize * row_bytes;
            rows.copy_from_slice(&blurred.as_raw()[start..start + rows.len()]);
        });
    output
}

pub(crate) fn blend_images(source: &RgbaImage, blurred: &RgbaImage, amount: f32) -> RgbaImage {
    let mut output = source.clone();
    output
        .as_mut()
        .par_chunks_mut(4 * 1024)
        .zip(blurred.as_raw().par_chunks(4 * 1024))
        .for_each(|(pixels, blurs)| {
            for (pixel, blur) in pixels.chunks_exact_mut(4).zip(blurs.chunks_exact(4)) {
                for channel in 0..3 {
                    pixel[channel] = (pixel[channel] as f32 * (1.0 - amount)
                        + blur[channel] as f32 * amount
                        + 0.5) as u8;
                }
            }
        });
    output
}

pub(crate) fn apply_unsharp(image: &mut RgbaImage, blurred: &RgbaImage, amount: f32) {
    image
        .as_mut()
        .par_chunks_mut(4 * 1024)
        .zip(blurred.as_raw().par_chunks(4 * 1024))
        .for_each(|(pixels, blurs)| {
            for (pixel, blur) in pixels.chunks_exact_mut(4).zip(blurs.chunks_exact(4)) {
                for channel in 0..3 {
                    let value = pixel[channel] as f32
                        + (pixel[channel] as f32 - blur[channel] as f32) * amount;
                    pixel[channel] = value.clamp(0.0, 255.0) as u8;
                }
            }
        });
}

/// Rotates by `degrees` about the center, zoomed in just enough that no
/// empty corner shows.
pub(crate) fn rotate_filled(image: DynamicImage, degrees: f32) -> DynamicImage {
    let source = to_rgba(image);
    let (width, height) = source.dimensions();
    let mut output = RgbaImage::new(width, height);
    let radians = degrees.to_radians();
    let (sin, cos) = radians.sin_cos();
    let aspect = width as f32 / height.max(1) as f32;
    let zoom = (cos.abs() + aspect * sin.abs())
        .max(cos.abs() + sin.abs() / aspect)
        .max(1.0);
    let cx = (width as f32 - 1.0) * 0.5;
    let cy = (height as f32 - 1.0) * 0.5;
    output
        .as_mut()
        .par_chunks_mut(width as usize * 4)
        .enumerate()
        .for_each(|(y, row)| {
            for (x, pixel) in row.chunks_exact_mut(4).enumerate() {
                let dx = (x as f32 - cx) / zoom;
                let dy = (y as f32 - cy) / zoom;
                let sx = cos * dx + sin * dy + cx;
                let sy = -sin * dx + cos * dy + cy;
                pixel.copy_from_slice(&sample_bilinear(&source, sx, sy).0);
            }
        });
    DynamicImage::ImageRgba8(output)
}

fn sample_bilinear(image: &RgbaImage, x: f32, y: f32) -> Rgba<u8> {
    let x = x.clamp(0.0, image.width().saturating_sub(1) as f32);
    let y = y.clamp(0.0, image.height().saturating_sub(1) as f32);
    let x0 = x.floor() as u32;
    let y0 = y.floor() as u32;
    let x1 = (x0 + 1).min(image.width() - 1);
    let y1 = (y0 + 1).min(image.height() - 1);
    let tx = x - x0 as f32;
    let ty = y - y0 as f32;
    let (p00, p10) = (image.get_pixel(x0, y0), image.get_pixel(x1, y0));
    let (p01, p11) = (image.get_pixel(x0, y1), image.get_pixel(x1, y1));
    let mut out = [0; 4];
    for (channel, value) in out.iter_mut().enumerate() {
        let top = p00[channel] as f32 * (1.0 - tx) + p10[channel] as f32 * tx;
        let bottom = p01[channel] as f32 * (1.0 - tx) + p11[channel] as f32 * tx;
        *value = (top * (1.0 - ty) + bottom * ty + 0.5) as u8;
    }
    Rgba(out)
}

/// `image` turned clockwise by `degrees` (a multiple of 90), flipped, and
/// cropped to `crop` (x, y, width, height in the turned image's pixels), as
/// `DynamicImage::rotate90` and friends, `fliph`, `flipv`, and `crop_imm`
/// give it, on every core. Other pixel layouts use those directly.
pub(crate) fn rearrange(
    image: DynamicImage,
    degrees: i32,
    flip: (bool, bool),
    crop: impl FnOnce(u32, u32) -> Option<(u32, u32, u32, u32)>,
) -> DynamicImage {
    let channels = match &image {
        DynamicImage::ImageRgb8(_) => 3,
        DynamicImage::ImageRgba8(_) => 4,
        _ => return rearrange_slowly(image, degrees, flip, crop),
    };
    let (width, height) = (image.width(), image.height());
    let turned = degrees == 90 || degrees == 270;
    let (out_width, out_height) = if turned {
        (height, width)
    } else {
        (width, height)
    };
    let (x0, y0, crop_width, crop_height) =
        crop(out_width, out_height).unwrap_or((0, 0, out_width, out_height));
    let source = image.as_bytes();
    let mut output = vec![0u8; crop_width as usize * crop_height as usize * channels];
    let (w, h) = (width as i64, height as i64);
    output
        .par_chunks_mut(crop_width as usize * channels)
        .enumerate()
        .for_each(|(row, line)| {
            // Where each output pixel comes from: undo the flips, then the turn.
            let mut y = i64::from(y0) + row as i64;
            if flip.1 {
                y = i64::from(out_height) - 1 - y;
            }
            for (column, pixel) in line.chunks_exact_mut(channels).enumerate() {
                let mut x = i64::from(x0) + column as i64;
                if flip.0 {
                    x = i64::from(out_width) - 1 - x;
                }
                let (sx, sy) = match degrees {
                    90 => (y, h - 1 - x),
                    180 => (w - 1 - x, h - 1 - y),
                    270 => (w - 1 - y, x),
                    _ => (x, y),
                };
                let at = (sy as usize * width as usize + sx as usize) * channels;
                pixel.copy_from_slice(&source[at..at + channels]);
            }
        });
    if channels == 3 {
        DynamicImage::ImageRgb8(
            image::RgbImage::from_raw(crop_width, crop_height, output).expect("whole pixels"),
        )
    } else {
        DynamicImage::ImageRgba8(
            RgbaImage::from_raw(crop_width, crop_height, output).expect("whole pixels"),
        )
    }
}

fn rearrange_slowly(
    mut image: DynamicImage,
    degrees: i32,
    flip: (bool, bool),
    crop: impl FnOnce(u32, u32) -> Option<(u32, u32, u32, u32)>,
) -> DynamicImage {
    image = match degrees {
        90 => image.rotate90(),
        180 => image.rotate180(),
        270 => image.rotate270(),
        _ => image,
    };
    if flip.0 {
        image = image.fliph();
    }
    if flip.1 {
        image = image.flipv();
    }
    match crop(image.width(), image.height()) {
        Some((x, y, width, height)) => image.crop_imm(x, y, width, height),
        None => image,
    }
}

/// `image` as RGBA, as `DynamicImage::to_rgba8` gives it, on every core for
/// RGB photos.
pub fn to_rgba(image: DynamicImage) -> RgbaImage {
    let DynamicImage::ImageRgb8(rgb) = image else {
        return image.into_rgba8();
    };
    let mut rgba = RgbaImage::new(rgb.width(), rgb.height());
    rgba.as_mut()
        .par_chunks_mut(4 * 4096)
        .zip(rgb.as_raw().par_chunks(3 * 4096))
        .for_each(|(out, source)| {
            for (pixel, from) in out.chunks_exact_mut(4).zip(source.chunks_exact(3)) {
                pixel[..3].copy_from_slice(from);
                pixel[3] = u8::MAX;
            }
        });
    rgba
}

#[cfg(test)]
mod tests {
    use super::*;

    fn textured(width: u32, height: u32) -> RgbaImage {
        RgbaImage::from_fn(width, height, |x, y| {
            let noise = (x.wrapping_mul(2_654_435_761) ^ y.wrapping_mul(40_503)) >> 24;
            Rgba([
                (x * 7 + noise) as u8,
                (y * 5) as u8 ^ noise as u8,
                (x + y) as u8,
                (200 + noise % 56) as u8,
            ])
        })
    }

    #[test]
    fn parallel_turns_flips_and_crops_match_the_image_crate() {
        let rgba = textured(37, 23);
        let rgb = DynamicImage::ImageRgba8(rgba.clone()).to_rgb8();
        for image in [DynamicImage::ImageRgba8(rgba), DynamicImage::ImageRgb8(rgb)] {
            for degrees in [0, 90, 180, 270] {
                for flip in [(false, false), (true, false), (false, true), (true, true)] {
                    let crop = |w: u32, h: u32| Some((3, 2, w - 7, h - 5));
                    let fast = rearrange(image.clone(), degrees, flip, crop);
                    let slow = rearrange_slowly(image.clone(), degrees, flip, crop);
                    assert!(fast == slow, "{degrees} {flip:?}");
                }
            }
            assert!(to_rgba(image.clone()) == image.to_rgba8());
        }
    }

    #[test]
    fn strip_blur_matches_the_whole_image_blur_exactly() {
        for (width, height) in [(37, 1031), (513, 777), (3, 400)] {
            let image = textured(width, height);
            for sigma in [1.1, 1.6] {
                let whole = DynamicImage::ImageRgba8(image.clone())
                    .blur(sigma)
                    .to_rgba8();
                assert!(
                    blur(&image, sigma) == whole,
                    "{width}x{height} sigma {sigma}"
                );
            }
        }
    }
}
