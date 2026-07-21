use super::*;

const MAX_SHADOW_MASK_EDGE: u32 = 2_048;
const MAX_SHADOW_MASK_RGBA_BYTES: u64 =
    MAX_SHADOW_MASK_EDGE as u64 * MAX_SHADOW_MASK_EDGE as u64 * 4;

fn shadow_mask_dimensions(width: u32, height: u32) -> [u32; 2] {
    let width = width.max(1);
    let height = height.max(1);
    if width.max(height) <= MAX_SHADOW_MASK_EDGE {
        return [width, height];
    }
    if width >= height {
        let scaled_height = (u64::from(height) * u64::from(MAX_SHADOW_MASK_EDGE)
            + u64::from(width) / 2)
            / u64::from(width);
        [
            MAX_SHADOW_MASK_EDGE,
            scaled_height.clamp(1, u64::from(MAX_SHADOW_MASK_EDGE)) as u32,
        ]
    } else {
        let scaled_width = (u64::from(width) * u64::from(MAX_SHADOW_MASK_EDGE)
            + u64::from(height) / 2)
            / u64::from(height);
        [
            scaled_width.clamp(1, u64::from(MAX_SHADOW_MASK_EDGE)) as u32,
            MAX_SHADOW_MASK_EDGE,
        ]
    }
}

pub(super) fn bounded_shadow_mask(source: &image::RgbaImage) -> image::RgbaImage {
    let [width, height] = shadow_mask_dimensions(source.width(), source.height());
    debug_assert!(u64::from(width) * u64::from(height) * 4 <= MAX_SHADOW_MASK_RGBA_BYTES);
    let mut mask =
        image::imageops::resize(source, width, height, image::imageops::FilterType::Triangle);
    for pixel in mask.pixels_mut() {
        pixel.0[..3].fill(255);
    }
    mask
}

pub(super) fn for_each_shadow_preview_sample(
    shadow: prism_core::DropShadow,
    layer_opacity: f32,
    mut visit: impl FnMut(Vec2, Color32),
) {
    let target_alpha = f32::from(shadow.color[3]) / 255.0 * layer_opacity.clamp(0.0, 1.0);
    if target_alpha <= 0.0 {
        return;
    }
    if shadow.blur_radius < 0.5 {
        visit(
            Vec2::new(shadow.offset_x, shadow.offset_y),
            Color32::from_rgba_unmultiplied(
                shadow.color[0],
                shadow.color[1],
                shadow.color[2],
                quantize_nonzero_alpha(target_alpha),
            ),
        );
        return;
    }
    // One quad per nonzero fixed-kernel tap keeps interaction cost independent of blur radius.
    let alphas = blurred_shadow_preview_alphas(target_alpha);
    for ((unit_x, unit_y, _), alpha) in prism_core::DROP_SHADOW_KERNEL.into_iter().zip(alphas) {
        if alpha == 0 {
            continue;
        }
        visit(
            Vec2::new(
                shadow.offset_x - unit_x * shadow.blur_radius,
                shadow.offset_y - unit_y * shadow.blur_radius,
            ),
            Color32::from_rgba_unmultiplied(
                shadow.color[0],
                shadow.color[1],
                shadow.color[2],
                alpha,
            ),
        );
    }
}

fn quantize_nonzero_alpha(alpha: f32) -> u8 {
    let quantized = (alpha * 255.0).round().clamp(0.0, 255.0) as u8;
    if alpha > 0.0 { quantized.max(1) } else { 0 }
}

fn blurred_shadow_preview_alphas(target_alpha: f32) -> [u8; prism_core::DROP_SHADOW_KERNEL.len()] {
    let mut alphas = [0; prism_core::DROP_SHADOW_KERNEL.len()];
    let mut outer_transmittance = 1.0_f32;
    for (index, (_, _, weight)) in prism_core::DROP_SHADOW_KERNEL
        .into_iter()
        .enumerate()
        .skip(1)
    {
        let exponent = weight as f32 / prism_core::DROP_SHADOW_KERNEL_TOTAL_WEIGHT as f32;
        let ideal = 1.0 - (1.0 - target_alpha).powf(exponent);
        let alpha = (ideal * 255.0).floor().clamp(0.0, 255.0) as u8;
        alphas[index] = alpha;
        outer_transmittance *= 1.0 - f32::from(alpha) / 255.0;
    }
    let center = if outer_transmittance <= f32::EPSILON {
        255
    } else {
        let residual = 1.0 - (1.0 - target_alpha) / outer_transmittance;
        quantize_nonzero_alpha(residual.clamp(0.0, 1.0))
    };
    alphas[0] = center;
    alphas
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colored_shadow_mask_has_an_exact_dimension_and_memory_ceiling() {
        assert_eq!(shadow_mask_dimensions(8_192, 8_192), [2_048, 2_048]);
        assert_eq!(shadow_mask_dimensions(8_192, 4_096), [2_048, 1_024]);
        assert_eq!(shadow_mask_dimensions(4_096, 8_192), [1_024, 2_048]);
        assert_eq!(shadow_mask_dimensions(1, 8_192), [1, 2_048]);
        assert_eq!(shadow_mask_dimensions(640, 360), [640, 360]);

        let dimensions = shadow_mask_dimensions(8_192, 8_192);
        let bytes = u64::from(dimensions[0]) * u64::from(dimensions[1]) * 4;
        assert_eq!(bytes, 16 * 1_024 * 1_024);
        assert_eq!(bytes, MAX_SHADOW_MASK_RGBA_BYTES);
    }

    #[test]
    fn colored_shadow_mask_preserves_alpha_and_whitens_rgb_in_place() {
        let source = image::RgbaImage::from_raw(
            2,
            2,
            vec![10, 20, 30, 0, 40, 50, 60, 64, 70, 80, 90, 128, 1, 2, 3, 255],
        )
        .unwrap();
        let mask = bounded_shadow_mask(&source);

        assert_eq!(mask.dimensions(), (2, 2));
        assert_eq!(
            mask.pixels().map(|pixel| pixel.0).collect::<Vec<_>>(),
            vec![
                [255, 255, 255, 0],
                [255, 255, 255, 64],
                [255, 255, 255, 128],
                [255, 255, 255, 255]
            ]
        );
    }

    #[test]
    fn blurred_shadow_preview_is_bounded_and_preserves_fully_overlapped_alpha() {
        let shadow = prism_core::DropShadow {
            color: [12, 24, 36, 160],
            offset_x: 28.0,
            offset_y: 34.0,
            blur_radius: 20.0,
        };
        let mut samples = Vec::new();
        for_each_shadow_preview_sample(shadow, 0.75, |offset, color| {
            samples.push((offset, color));
        });

        assert_eq!(samples.len(), prism_core::DROP_SHADOW_KERNEL.len());
        assert!(
            samples
                .iter()
                .any(|(offset, _)| *offset == Vec2::new(28.0, 34.0))
        );
        let actual_tint = samples[0].1.to_srgba_unmultiplied();
        assert!(
            actual_tint[..3]
                .iter()
                .zip([12, 24, 36])
                .all(|(actual, expected)| actual.abs_diff(expected) <= 3)
        );
        let composed_alpha = samples.iter().fold(0.0_f32, |alpha, (_, color)| {
            let source = f32::from(color.a()) / 255.0;
            alpha + source * (1.0 - alpha)
        });
        let expected_alpha = f32::from(shadow.color[3]) / 255.0 * 0.75;
        assert!((composed_alpha - expected_alpha).abs() <= 1.0 / 255.0);
    }

    #[test]
    fn low_alpha_blurred_shadows_remain_visible_with_bounded_composite_error() {
        for source_alpha in 1..=8 {
            for opacity in [1.0, 0.5, 0.1, 0.01] {
                let shadow = prism_core::DropShadow {
                    color: [80, 120, 160, source_alpha],
                    blur_radius: 20.0,
                    ..prism_core::DropShadow::default()
                };
                let mut alphas = Vec::new();
                for_each_shadow_preview_sample(shadow, opacity, |_, color| {
                    alphas.push(color.a());
                });

                assert!(alphas.iter().any(|alpha| *alpha > 0));
                assert!(alphas.len() <= prism_core::DROP_SHADOW_KERNEL.len());
                let composed = alphas.iter().fold(0.0_f32, |alpha, source| {
                    alpha + f32::from(*source) / 255.0 * (1.0 - alpha)
                });
                let expected = f32::from(source_alpha) / 255.0 * opacity;
                assert!(
                    (composed - expected).abs() <= 1.0 / 255.0 + f32::EPSILON,
                    "source alpha {source_alpha}, opacity {opacity}, composed {composed}, expected {expected}"
                );
            }
        }
    }

    #[test]
    fn sharp_shadow_preview_is_one_unblurred_offset_sample() {
        let shadow = prism_core::DropShadow {
            color: [8, 16, 24, 128],
            offset_x: -12.0,
            offset_y: 7.0,
            blur_radius: 0.0,
        };
        let mut samples = Vec::new();
        for_each_shadow_preview_sample(shadow, 0.5, |offset, color| {
            samples.push((offset, color));
        });

        assert_eq!(samples.len(), 1);
        assert_eq!(samples[0].0, Vec2::new(-12.0, 7.0));
        assert_eq!(samples[0].1, Color32::from_rgba_unmultiplied(8, 16, 24, 64));
    }
}
