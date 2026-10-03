//! Blending one source pixel onto the canvas.
use image::{Rgba, RgbaImage};

use crate::blend_rgb;

pub(crate) fn composite_blended_pixel(
    canvas: &mut RgbaImage,
    source_pixel: [u8; 4],
    blend_mode: crate::BlendMode,
    alpha: f32,
    x: u32,
    y: u32,
) {
    let destination = canvas.get_pixel(x, y).0;
    let output = blend_pixel(destination, source_pixel, blend_mode, alpha);
    canvas.put_pixel(x, y, Rgba(output));
}

/// `source_pixel` blended over `destination` at `alpha`.
pub(crate) fn blend_pixel(
    destination: [u8; 4],
    source_pixel: [u8; 4],
    blend_mode: crate::BlendMode,
    alpha: f32,
) -> [u8; 4] {
    let blended = blend_rgb(source_pixel, destination, blend_mode);
    let destination_alpha = destination[3] as f32 / 255.0;
    let output_alpha = alpha + destination_alpha * (1.0 - alpha);
    let mut output = [0; 4];
    for channel in 0..3 {
        let value = if output_alpha > 0.0 {
            (source_pixel[channel] as f32 * alpha * (1.0 - destination_alpha)
                + blended[channel] as f32 * alpha * destination_alpha
                + destination[channel] as f32 * destination_alpha * (1.0 - alpha))
                / output_alpha
        } else {
            0.0
        };
        output[channel] = round_byte(value);
    }
    output[3] = round_byte(output_alpha * 255.0);
    output
}

/// Rounds to the nearest byte, halves up, without a `round` library call
/// (the baseline x86-64 target has no rounding instruction).
#[inline]
pub(crate) fn round_byte(value: f32) -> u8 {
    (value.clamp(0.0, 255.0) + 0.5) as u8
}
