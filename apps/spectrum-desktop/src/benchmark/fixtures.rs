//! The benchmark library: a 24-megapixel photo, and a canvas holding it fit
//! to the canvas under a text layer with a glow.
use super::Fixtures;
use anyhow::{Context, Result};
use spectrum_assets::Service;
use spectrum_canvas::{Command, Glow, LayerStyle, Transform};
use std::path::Path;

/// The photo's size: a typical full-frame camera.
const PHOTO: (u32, u32) = (6000, 4000);

pub(super) fn make(directory: &Path, library: &Path, photo: Option<&Path>) -> Result<Fixtures> {
    let photo_path = match photo {
        Some(photo) => photo.to_path_buf(),
        None => generate(directory)?,
    };
    let photo_width = spectrum_image::Image::import(&photo_path)?.width;
    let mut service = Service::open(library)?;
    let project = service.library.create_project("Benchmark")?.id;
    let (_, mut assets) = service.import_into(vec![photo_path], Some(project))?;
    let photo = assets.pop().context("the photo was not imported")?.id;
    let canvas = service.create_canvas("Benchmark canvas".into(), 1920, 1080)?;
    service.library.add_to_project(project, &[canvas.id])?;
    let photo_layer = service.place(canvas.id, photo)?;
    let scale = 1920.0 / photo_width as f32;
    let outputs = service.edit_canvas(
        canvas.id,
        vec![
            Command::SetTransform {
                id: photo_layer,
                transform: Transform {
                    x: 0.0,
                    y: 0.0,
                    scale_x: scale,
                    scale_y: scale,
                    rotation: 0.0,
                },
            },
            Command::AddText {
                text: "Spectrum benchmark".into(),
                name: None,
                font_size: 140.0,
                color: [255, 255, 255, 255],
                x: 160.0,
                y: 420.0,
                shaping: Default::default(),
            },
        ],
    )?;
    let text_layer = *outputs
        .last()
        .and_then(|output| output.layer_ids.first())
        .context("the text layer was not added")?;
    service.edit_canvas(
        canvas.id,
        vec![Command::SetLayerStyle {
            id: text_layer,
            style: LayerStyle {
                outer_glow: Some(Glow {
                    size: 24.0,
                    ..Glow::default()
                }),
                ..LayerStyle::default()
            },
        }],
    )?;
    Ok(Fixtures {
        directory: directory.to_path_buf(),
        project,
        photo,
        canvas: canvas.id,
        photo_layer,
        text_layer,
    })
}

/// A textured 24-megapixel JPEG: not a flat gradient, so decoding and
/// adjusting it cost what a real photo costs.
fn generate(directory: &Path) -> Result<std::path::PathBuf> {
    let path = directory.join("photo.jpg");
    image::RgbImage::from_fn(PHOTO.0, PHOTO.1, |x, y| {
        let wave = ((x as f32 * 0.013).sin() + (y as f32 * 0.021).cos()) * 40.0;
        let grain = ((x.wrapping_mul(2_654_435_761) ^ y.wrapping_mul(40_503)) >> 24) as f32 * 0.25;
        image::Rgb([
            (90.0 + wave + grain + y as f32 * 0.02).clamp(0.0, 255.0) as u8,
            (110.0 + wave * 0.5 + grain + x as f32 * 0.01).clamp(0.0, 255.0) as u8,
            (140.0 - wave + grain).clamp(0.0, 255.0) as u8,
        ])
    })
    .save(&path)?;
    Ok(path)
}
