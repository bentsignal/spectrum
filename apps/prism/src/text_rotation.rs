use image::{DynamicImage, RgbaImage, imageops::FilterType};

use crate::Transform;

pub(crate) fn transform_text_layer(
    image: DynamicImage,
    transform: Transform,
    visual_pivot: (f32, f32),
) -> (RgbaImage, (f32, f32)) {
    let source_width = image.width().max(1) as f32;
    let source_height = image.height().max(1) as f32;
    let width = (source_width * transform.scale_x).round().max(1.0) as u32;
    let height = (source_height * transform.scale_y).round().max(1.0) as u32;
    let scaled = image
        .resize_exact(width, height, FilterType::Triangle)
        .to_rgba8();
    if transform.rotation.abs() < 0.01 {
        return (scaled, (0.0, 0.0));
    }
    let pivot = (
        visual_pivot.0 * width as f32 / source_width,
        visual_pivot.1 * height as f32 / source_height,
    );
    rotate_rgba_about(&scaled, transform.rotation, pivot)
}

fn rotate_rgba_about(
    source: &RgbaImage,
    degrees: f32,
    pivot: (f32, f32),
) -> (RgbaImage, (f32, f32)) {
    let (sin, cos) = crate::transform_math::rotation_sin_cos(degrees);
    let rotate = |point: (f32, f32)| {
        let dx = point.0 - pivot.0;
        let dy = point.1 - pivot.1;
        (pivot.0 + dx * cos - dy * sin, pivot.1 + dx * sin + dy * cos)
    };
    let corners = [
        rotate((0.0, 0.0)),
        rotate((source.width() as f32, 0.0)),
        rotate((source.width() as f32, source.height() as f32)),
        rotate((0.0, source.height() as f32)),
    ];
    let min_x = (corners
        .iter()
        .map(|point| point.0)
        .fold(f32::INFINITY, f32::min)
        + 0.0001)
        .floor();
    let min_y = (corners
        .iter()
        .map(|point| point.1)
        .fold(f32::INFINITY, f32::min)
        + 0.0001)
        .floor();
    let max_x = (corners
        .iter()
        .map(|point| point.0)
        .fold(f32::NEG_INFINITY, f32::max)
        - 0.0001)
        .ceil();
    let max_y = (corners
        .iter()
        .map(|point| point.1)
        .fold(f32::NEG_INFINITY, f32::max)
        - 0.0001)
        .ceil();
    let output_width = (max_x - min_x).max(1.0) as u32;
    let output_height = (max_y - min_y).max(1.0) as u32;
    let mut output = RgbaImage::new(output_width, output_height);
    for y in 0..output_height {
        for x in 0..output_width {
            let world_x = min_x + x as f32 + 0.5;
            let world_y = min_y + y as f32 + 0.5;
            let dx = world_x - pivot.0;
            let dy = world_y - pivot.1;
            let source_x = cos * dx + sin * dy + pivot.0 - 0.5;
            let source_y = -sin * dx + cos * dy + pivot.1 - 0.5;
            if source_x >= 0.0
                && source_y >= 0.0
                && source_x < source.width() as f32
                && source_y < source.height() as f32
            {
                let sample_x = source_x
                    .round()
                    .clamp(0.0, source.width().saturating_sub(1) as f32)
                    as u32;
                let sample_y = source_y
                    .round()
                    .clamp(0.0, source.height().saturating_sub(1) as f32)
                    as u32;
                output.put_pixel(x, y, *source.get_pixel(sample_x, sample_y));
            }
        }
    }
    (output, (min_x, min_y))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arbitrary_text_pivot_keeps_its_document_position_at_right_angles() {
        let source = DynamicImage::ImageRgba8(RgbaImage::new(100, 60));
        let (rotated, offset) = transform_text_layer(
            source,
            Transform {
                rotation: 90.0,
                ..Transform::default()
            },
            (20.0, 30.0),
        );

        assert_eq!((rotated.width(), rotated.height()), (60, 100));
        assert_eq!(offset, (-10.0, 10.0));
        let pivot_in_rotated_image = (20.0 - offset.0, 30.0 - offset.1);
        assert_eq!(
            (
                offset.0 + pivot_in_rotated_image.0,
                offset.1 + pivot_in_rotated_image.1
            ),
            (20.0, 30.0)
        );
    }
}
