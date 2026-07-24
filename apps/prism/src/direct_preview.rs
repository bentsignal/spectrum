use anyhow::{Result, anyhow};
use image::{DynamicImage, RgbaImage};

use crate::{
    Document, RasterSourceResolver, RenderRegion, render_document_region_scaled,
    render_document_region_scaled_with_sources,
};

/// Direct manipulation caps exact compositor fanout to avoid oversubscribing
/// the UI process while independent output strips render in parallel.
pub const DIRECT_PREVIEW_STRIPS: usize = 4;
pub const DIRECT_PREVIEW_MAX_OUTPUT_PIXELS: u64 = 4_096 * 4_096;

pub fn render_direct_preview_region_scaled(
    document: &Document,
    scale: f32,
    region: RenderRegion,
) -> Result<DynamicImage> {
    render_strips(document, scale, region, None)
}

pub fn render_direct_preview_region_scaled_with_sources(
    document: &Document,
    scale: f32,
    region: RenderRegion,
    raster_sources: &dyn RasterSourceResolver,
) -> Result<DynamicImage> {
    render_strips(document, scale, region, Some(raster_sources))
}

fn render_strips(
    document: &Document,
    scale: f32,
    region: RenderRegion,
    raster_sources: Option<&dyn RasterSourceResolver>,
) -> Result<DynamicImage> {
    if region.width == 0 || region.height == 0 {
        return render_one(document, scale, region, raster_sources);
    }
    let output_pixels = u64::from(region.width)
        .checked_mul(u64::from(region.height))
        .ok_or_else(|| anyhow!("direct preview output area overflowed"))?;
    if output_pixels > DIRECT_PREVIEW_MAX_OUTPUT_PIXELS {
        return Err(anyhow!(
            "direct preview output exceeds the aggregate viewport pixel bound"
        ));
    }
    let strip_count = DIRECT_PREVIEW_STRIPS.min(region.height as usize);
    let rows_per_strip = region.height / strip_count as u32;
    let extra_rows = region.height % strip_count as u32;
    let strips = std::thread::scope(|scope| {
        let mut handles = Vec::with_capacity(strip_count);
        let mut relative_y = 0;
        for index in 0..strip_count {
            let height = rows_per_strip + u32::from(index < extra_rows as usize);
            let strip = RenderRegion {
                x: region.x,
                y: region.y + relative_y,
                width: region.width,
                height,
            };
            handles.push((
                relative_y,
                scope.spawn(move || render_one(document, scale, strip, raster_sources)),
            ));
            relative_y += height;
        }
        handles
            .into_iter()
            .map(|(relative_y, handle)| {
                handle
                    .join()
                    .map_err(|_| anyhow!("direct preview compositor worker panicked"))?
                    .map(|image| (relative_y, image.into_rgba8()))
            })
            .collect::<Result<Vec<_>>>()
    })?;

    let mut stitched = RgbaImage::new(region.width, region.height);
    for (relative_y, strip) in strips {
        image::imageops::replace(&mut stitched, &strip, 0, i64::from(relative_y));
    }
    Ok(DynamicImage::ImageRgba8(stitched))
}

fn render_one(
    document: &Document,
    scale: f32,
    region: RenderRegion,
    raster_sources: Option<&dyn RasterSourceResolver>,
) -> Result<DynamicImage> {
    if let Some(raster_sources) = raster_sources {
        render_document_region_scaled_with_sources(document, scale, region, raster_sources)
    } else {
        render_document_region_scaled(document, scale, region)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BlendMode, Layer, LayerKind, Transform};

    fn assert_striped_parity(document: &Document, region: RenderRegion) {
        let single = render_document_region_scaled(document, 1.0, region)
            .unwrap()
            .into_rgba8();
        let striped = render_direct_preview_region_scaled(document, 1.0, region)
            .unwrap()
            .into_rgba8();
        assert_eq!(striped, single);
    }

    #[test]
    fn exact_strips_match_single_region_for_move_rotate_resize_and_uneven_seams() {
        let mut document = Document::new("Direct preview oracle", 519, 17);
        document.background = [0; 4];
        document.layers.push(Layer {
            id: 1,
            opacity: 0.5,
            blend_mode: BlendMode::Dissolve,
            dissolve_seed: 0x1234_5678,
            kind: LayerKind::Rectangle {
                width: 260,
                height: 11,
                color: [35, 145, 225, 173],
                corner_radius: 0.0,
            },
            ..Layer::default()
        });
        let region = RenderRegion {
            x: 0,
            y: 0,
            width: 519,
            height: 17,
        };
        for transform in [
            Transform {
                x: 253.0,
                y: 3.0,
                ..Transform::default()
            },
            Transform {
                x: 253.0,
                y: 3.0,
                rotation: 17.0,
                ..Transform::default()
            },
            Transform {
                x: 149.0,
                y: 4.0,
                scale_x: 1.4,
                scale_y: 0.8,
                ..Transform::default()
            },
        ] {
            document.layers[0].transform = transform;
            assert_striped_parity(&document, region);
        }
    }

    #[test]
    fn one_pixel_regions_do_not_drop_or_duplicate_rows() {
        let mut document = Document::new("One pixel strip", 3, 5);
        document.background = [7, 11, 13, 255];
        document.layers.push(Layer {
            opacity: 0.5,
            blend_mode: BlendMode::Dissolve,
            dissolve_seed: 91,
            kind: LayerKind::Rectangle {
                width: 3,
                height: 5,
                color: [230, 80, 150, 255],
                corner_radius: 0.0,
            },
            ..Layer::default()
        });
        for y in 0..5 {
            assert_striped_parity(
                &document,
                RenderRegion {
                    x: 1,
                    y,
                    width: 1,
                    height: 1,
                },
            );
        }
    }

    #[test]
    fn fanout_cannot_bypass_the_aggregate_viewport_bound() {
        let document = Document::new("Bounded direct preview", 8_193, 2_049);
        let error = render_direct_preview_region_scaled(
            &document,
            1.0,
            RenderRegion {
                x: 0,
                y: 0,
                width: 8_193,
                height: 2_049,
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains("aggregate viewport pixel bound"));
    }
}
