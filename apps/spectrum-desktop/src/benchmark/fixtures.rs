//! The benchmark library: a 24-megapixel photo; a 45-megapixel photo with a
//! full editing pass on every tab; a canvas holding the first photo fit to
//! the canvas under a text layer with a glow; and a heavy 4K canvas of
//! twelve photos, text with effects, and shapes.
use super::Fixtures;
use anyhow::{Context, Result};
use spectrum_assets::Service;
use spectrum_canvas::{BlendMode, Command, DropShadow, Glow, LayerStyle, Transform};
use spectrum_image::{
    Adjustments, ColorGrade, CropRect, CurvePoint, HslBand, SpotRemoval, ToneCurve,
};
use std::path::Path;

/// The photo's size: a typical full-frame camera.
const PHOTO: (u32, u32) = (6000, 4000);
/// The edited photo's size: a high-resolution camera.
const LARGE_PHOTO: (u32, u32) = (8256, 5504);
/// The size of the heavy canvas's other photos: a phone camera.
const PHONE_PHOTO: (u32, u32) = (4000, 3000);

pub(super) fn make(directory: &Path, library: &Path, photo: Option<&Path>) -> Result<Fixtures> {
    let photo_path = match photo {
        Some(photo) => photo.to_path_buf(),
        None => generate(&directory.join("photo.jpg"), PHOTO, 0)?,
    };
    let large_path = match photo {
        Some(photo) => photo.to_path_buf(),
        None => generate(&directory.join("large.jpg"), LARGE_PHOTO, 1)?,
    };
    let photo_width = spectrum_image::Image::import(&photo_path)?.width;
    let mut service = Service::open(library)?;
    let project = service.library.create_project("Benchmark")?.id;
    let (_, assets) = service.import_into(vec![photo_path, large_path], Some(project))?;
    let [photo, edited_photo] = [0, 1].map(|index| assets.get(index).map(|asset| asset.id));
    let photo = photo.context("the photo was not imported")?;
    let edited_photo = edited_photo.context("the large photo was not imported")?;
    service.set_adjustments(edited_photo, full_pass())?;
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
    let (heavy_canvas, heavy_layer) =
        heavy(&mut service, directory, project, &[photo, edited_photo])?;
    let blend_canvas = blend(&mut service, project, photo, photo_width)?;
    Ok(Fixtures {
        directory: directory.to_path_buf(),
        project,
        photo,
        edited_photo,
        canvas: canvas.id,
        photo_layer,
        text_layer,
        heavy_canvas,
        heavy_layer,
        blend_canvas,
    })
}

/// A canvas drawn as one image: the photo under a shape that multiplies
/// into it.
fn blend(
    service: &mut Service,
    project: spectrum_library::ProjectId,
    photo: spectrum_library::AssetId,
    photo_width: u32,
) -> Result<spectrum_library::AssetId> {
    let canvas = service.create_canvas("Blend canvas".into(), 1920, 1080)?;
    service.library.add_to_project(project, &[canvas.id])?;
    let layer = service.place(canvas.id, photo)?;
    let scale = 1920.0 / photo_width as f32;
    let outputs = service.edit_canvas(
        canvas.id,
        vec![
            Command::SetTransform {
                id: layer,
                transform: Transform {
                    x: 0.,
                    y: 0.,
                    scale_x: scale,
                    scale_y: scale,
                    rotation: 0.,
                },
            },
            Command::AddRectangle {
                name: None,
                width: 1200,
                height: 600,
                color: [255, 170, 90, 255],
                corner_radius: 40.,
                x: 360.,
                y: 240.,
            },
        ],
    )?;
    let shape = *outputs
        .last()
        .and_then(|output| output.layer_ids.first())
        .context("the shape was not added")?;
    service.edit_canvas(
        canvas.id,
        vec![Command::SetBlendMode {
            id: shape,
            blend_mode: BlendMode::Multiply,
        }],
    )?;
    Ok(canvas.id)
}

/// A 4K canvas laid out like a moodboard: twelve photos in a grid (the two
/// above, six phone photos, and four placed twice), four text layers with
/// shadows and glows, and two shapes with shadows. Returns it and the id of
/// a photo layer to drag.
fn heavy(
    service: &mut Service,
    directory: &Path,
    project: spectrum_library::ProjectId,
    photos: &[spectrum_library::AssetId],
) -> Result<(spectrum_library::AssetId, u64)> {
    let paths = (0..6)
        .map(|index| {
            generate(
                &directory.join(format!("phone-{index}.jpg")),
                PHONE_PHOTO,
                2 + index,
            )
        })
        .collect::<Result<Vec<_>>>()?;
    let (_, phone) = service.import_into(paths, Some(project))?;
    let mut sources: Vec<_> = photos.to_vec();
    sources.extend(phone.iter().map(|asset| asset.id));
    sources.extend(phone.iter().take(4).map(|asset| asset.id));
    let canvas = service.create_canvas("Heavy canvas".into(), 3840, 2160)?;
    service.library.add_to_project(project, &[canvas.id])?;
    let mut commands = Vec::new();
    let mut layers = Vec::new();
    for (index, source) in sources.iter().enumerate() {
        let layer = service.place(canvas.id, *source)?;
        let width = service.image(*source)?.width as f32;
        let (column, row) = ((index % 4) as f32, (index / 4) as f32);
        commands.push(Command::SetTransform {
            id: layer,
            transform: Transform {
                x: column * 960. + 30.,
                y: row * 720. + 30.,
                scale_x: 900. / width,
                scale_y: 900. / width,
                rotation: 0.,
            },
        });
        layers.push(layer);
    }
    for (index, text) in ["Moodboard", "Spring", "Light", "Texture"]
        .iter()
        .enumerate()
    {
        commands.push(Command::AddText {
            text: (*text).into(),
            name: None,
            font_size: 120. + index as f32 * 20.,
            color: [255, 255, 255, 255],
            x: 200. + index as f32 * 900.,
            y: 300. + index as f32 * 400.,
            shaping: Default::default(),
        });
    }
    for index in 0..2 {
        commands.push(Command::AddRectangle {
            name: None,
            width: 700,
            height: 260,
            color: [240, 200, 120, 220],
            corner_radius: 24.,
            x: 400. + index as f32 * 1800.,
            y: 1700.,
        });
    }
    let outputs = service.edit_canvas(canvas.id, commands)?;
    let styled: Vec<u64> = outputs
        .iter()
        .flat_map(|output| output.layer_ids.iter().copied())
        .collect();
    let styles = styled
        .iter()
        .enumerate()
        .map(|(index, id)| Command::SetLayerStyle {
            id: *id,
            style: LayerStyle {
                drop_shadow: Some(DropShadow::default()),
                outer_glow: (index % 2 == 0).then(|| Glow {
                    size: 18.0,
                    ..Glow::default()
                }),
                ..LayerStyle::default()
            },
        })
        .collect();
    service.edit_canvas(canvas.id, styles)?;
    Ok((canvas.id, layers[5]))
}

/// What a person's full editing pass sets: every tab, a quarter turn, a
/// straightened horizon, a crop, and spot removal.
fn full_pass() -> Adjustments {
    let band = |hue, saturation, luminance| HslBand {
        hue,
        saturation,
        luminance,
    };
    let mut adjustments = Adjustments {
        exposure: 0.35,
        temperature: 8.0,
        tint: -4.0,
        contrast: 18.0,
        highlights: -42.0,
        shadows: 35.0,
        whites: 12.0,
        blacks: -14.0,
        texture: 12.0,
        clarity: 18.0,
        dehaze: 8.0,
        vibrance: 20.0,
        saturation: -5.0,
        vignette: -22.0,
        sharpening: 45.0,
        noise_reduction: 25.0,
        rotation: 90,
        straighten: 1.8,
        crop: Some(CropRect {
            x: 0.04,
            y: 0.06,
            width: 0.9,
            height: 0.88,
        }),
        spots: (0..12)
            .map(|index| SpotRemoval {
                x: 0.1 + index as f32 * 0.07,
                y: 0.3 + (index % 3) as f32 * 0.15,
                radius: 0.012,
                opacity: 1.0,
            })
            .collect(),
        ..Adjustments::default()
    };
    adjustments.hsl.red = band(5.0, -10.0, 4.0);
    adjustments.hsl.orange = band(-6.0, 8.0, 12.0);
    adjustments.hsl.yellow = band(-10.0, -20.0, 0.0);
    adjustments.hsl.green = band(15.0, -35.0, -10.0);
    adjustments.hsl.aqua = band(0.0, -15.0, 0.0);
    adjustments.hsl.blue = band(-8.0, -25.0, -18.0);
    adjustments.color_grading.shadows = ColorGrade {
        hue: 210.0,
        saturation: 18.0,
        luminance: -4.0,
    };
    adjustments.color_grading.midtones = ColorGrade {
        hue: 35.0,
        saturation: 8.0,
        luminance: 0.0,
    };
    adjustments.color_grading.highlights = ColorGrade {
        hue: 45.0,
        saturation: 14.0,
        luminance: 3.0,
    };
    adjustments.color_grading.balance = 10.0;
    let point = |x, y| CurvePoint { x, y };
    adjustments.curves.master = ToneCurve {
        points: vec![
            point(0.0, 0.04),
            point(0.25, 0.22),
            point(0.75, 0.8),
            point(1.0, 0.97),
        ],
    };
    adjustments.curves.blue = ToneCurve {
        points: vec![point(0.0, 0.03), point(0.5, 0.5), point(1.0, 1.0)],
    };
    adjustments
}

/// A textured JPEG: not a flat gradient, so decoding and adjusting it cost
/// what a real photo costs.
fn generate(path: &Path, (width, height): (u32, u32), seed: u32) -> Result<std::path::PathBuf> {
    let path = path.to_path_buf();
    image::RgbImage::from_fn(width, height, |x, y| {
        let (x, y) = (x + seed * 997, y + seed * 613);
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
