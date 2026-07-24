use std::time::{SystemTime, UNIX_EPOCH};

use image::{GenericImageView, Rgba};
use sha2::Digest;

use crate::*;

fn test_directory(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("prism-pixel-mask-{label}-{stamp}"))
}

fn test_actor() -> spectrum_revisions::Actor {
    spectrum_revisions::Actor {
        id: "person:pixel-mask-test".into(),
        display_name: "Pixel mask test".into(),
        kind: spectrum_revisions::ActorKind::Human,
    }
}

fn find_opaque_color(image: &image::RgbaImage, red: bool) -> (u32, u32) {
    image
        .enumerate_pixels()
        .find_map(|(x, y, pixel)| {
            let matches = pixel[3] > 240
                && if red {
                    pixel[0] > 220 && pixel[2] < 30
                } else {
                    pixel[2] > 220 && pixel[0] < 30
                };
            matches.then_some((x, y))
        })
        .expect("fixture must contain an opaque target color")
}

#[test]
fn magic_wand_delete_is_mask_only_one_revision_and_round_trips_two_edits() {
    let directory = test_directory("magic-wand-delete");
    std::fs::create_dir_all(&directory).unwrap();
    let source = directory.join("two-colors.png");
    image::RgbaImage::from_fn(12, 8, |x, _| {
        if x < 6 {
            Rgba([240, 10, 20, 255])
        } else {
            Rgba([10, 20, 240, 255])
        }
    })
    .save(&source)
    .unwrap();
    let source = std::fs::canonicalize(source).unwrap();
    let original_bytes = std::fs::read(&source).unwrap();
    let original_hash = sha2::Sha256::digest(&original_bytes);
    let project = directory.join("delete.prism");
    let session = spectrum_revisions::SessionId::new();
    let mut document = Document::new("Delete pixels", 64, 48);
    document.background = [0; 4];
    let mut workspace =
        Workspace::create_durable(document, &project, test_actor(), session).unwrap();
    workspace
        .execute(Command::AddRaster {
            path: source.clone(),
            name: Some("Source".into()),
            x: 24.0,
            y: 16.0,
        })
        .unwrap();
    workspace
        .execute(Command::SetTransform {
            id: 1,
            transform: Transform {
                x: 24.0,
                y: 16.0,
                scale_x: 1.5,
                scale_y: 1.5,
                rotation: 23.0,
            },
        })
        .unwrap();

    let initial = render_document(&workspace.document, None)
        .unwrap()
        .into_rgba8();
    let red = find_opaque_color(&initial, true);
    workspace
        .execute(Command::MagicWandSelection {
            x: red.0,
            y: red.1,
            tolerance: 8,
            contiguous: true,
            antialias: false,
            resolved_selection: None,
        })
        .unwrap();
    let red_selection = workspace.document.selection.clone();
    let revisions_before_delete = workspace.history().unwrap().unwrap().revisions.len();
    workspace
        .execute(Command::DeleteSelectedPixels { id: 1 })
        .unwrap();
    assert_eq!(
        workspace.history().unwrap().unwrap().revisions.len(),
        revisions_before_delete + 1
    );
    assert_eq!(workspace.document.selection, red_selection);
    assert_eq!(std::fs::read(&source).unwrap(), original_bytes);
    assert_eq!(
        sha2::Sha256::digest(std::fs::read(&source).unwrap()),
        original_hash
    );
    let after_red = render_document(&workspace.document, None)
        .unwrap()
        .into_rgba8();
    assert!(
        after_red.pixels().filter(|pixel| pixel[0] > 220).count()
            < initial.pixels().filter(|pixel| pixel[0] > 220).count()
    );
    let blue = find_opaque_color(&after_red, false);
    workspace
        .execute(Command::MagicWandSelection {
            x: blue.0,
            y: blue.1,
            tolerance: 8,
            contiguous: true,
            antialias: false,
            resolved_selection: None,
        })
        .unwrap();
    workspace
        .execute(Command::DeleteSelectedPixels { id: 1 })
        .unwrap();
    let after_blue = render_document(&workspace.document, None)
        .unwrap()
        .into_rgba8();
    assert!(
        after_blue.pixels().filter(|pixel| pixel[2] > 220).count()
            < after_red.pixels().filter(|pixel| pixel[2] > 220).count()
    );
    let final_document = workspace.document.clone();
    let mut reopened_document = final_document.clone();
    if let LayerKind::Raster { original_path, .. } = &mut reopened_document.layers[0].kind {
        *original_path = None;
    }
    let final_history = workspace.history().unwrap().unwrap().revisions.len();
    assert!(
        workspace
            .execute(Command::DeleteSelectedPixels { id: 1 })
            .is_err()
    );
    assert_eq!(
        workspace.history().unwrap().unwrap().revisions.len(),
        final_history
    );
    assert_eq!(workspace.document, final_document);

    workspace.execute(Command::Undo).unwrap();
    assert_eq!(
        render_document(&workspace.document, None)
            .unwrap()
            .into_rgba8(),
        after_red
    );
    workspace.execute(Command::Redo).unwrap();
    assert_eq!(workspace.document, reopened_document);
    drop(workspace);

    let connection = rusqlite::Connection::open(&project).unwrap();
    let operation_version: u32 = connection
        .query_row(
            "SELECT version FROM operation_payloads WHERE instr(CAST(bytes AS TEXT), 'delete_selected_pixels') > 0 LIMIT 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(operation_version, 12);
    drop(connection);

    let mut reopened = Workspace::open_as(&project, test_actor(), session).unwrap();
    assert_eq!(reopened.document, reopened_document);
    let reopened_full = render_document(&reopened.document, None)
        .unwrap()
        .into_rgba8();
    let reopened_region = render_document_region_scaled(
        &reopened.document,
        1.0,
        RenderRegion {
            x: 0,
            y: 0,
            width: reopened.document.width,
            height: reopened.document.height,
        },
    )
    .unwrap()
    .into_rgba8();
    assert_eq!(reopened_region, reopened_full);
    let export = directory.join("deleted.png");
    export_document(&reopened.document, &export, 92).unwrap();
    assert_eq!(image::open(&export).unwrap().into_rgba8(), reopened_full);
    reopened.execute(Command::Undo).unwrap();
    assert_eq!(
        render_document(&reopened.document, None)
            .unwrap()
            .into_rgba8(),
        after_red
    );
    reopened.execute(Command::Redo).unwrap();
    assert_eq!(reopened.document, reopened_document);
    let transfer = LayerTransfer::from_document(&reopened.document, 1).unwrap();
    assert_eq!(transfer.version, 7);
    assert_eq!(
        LayerTransfer::from_json(&transfer.to_json().unwrap()).unwrap(),
        transfer
    );
    assert_eq!(std::fs::read(&source).unwrap(), original_bytes);
    drop(reopened);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn deleting_selection_multiplies_existing_soft_mask_and_survives_geometry_changes() {
    let directory = test_directory("soft-mask-composition");
    std::fs::create_dir_all(&directory).unwrap();
    let source = directory.join("opaque.png");
    image::RgbaImage::from_pixel(2, 1, Rgba([210, 90, 30, 255]))
        .save(&source)
        .unwrap();
    let mut document = Document::new("Soft mask", 2, 1);
    crate::command_apply::apply_command(
        &mut document,
        Command::AddRaster {
            path: std::fs::canonicalize(source).unwrap(),
            name: None,
            x: 0.0,
            y: 0.0,
        },
    )
    .unwrap();
    document.layers[0].pixel_mask = Some(PixelMask::new(2, 1, vec![128, 64]));
    document.selection = Some(Selection::ColorMask {
        x: 0,
        y: 0,
        width: 2,
        height: 1,
        alpha: vec![128, 255].into(),
    });
    crate::pixel_masks::delete_selected_raster_pixels(&mut document, 1).unwrap();
    assert_eq!(
        document.layers[0]
            .pixel_mask
            .as_ref()
            .unwrap()
            .alpha
            .as_ref(),
        [64, 0]
    );

    let before = render_layer_preview(&document.layers[0], None)
        .unwrap()
        .into_rgba8();
    assert_eq!(
        before.pixels().map(|pixel| pixel[3]).collect::<Vec<_>>(),
        [64, 0]
    );
    crate::command_apply::apply_command(
        &mut document,
        Command::AdjustLayer {
            id: 1,
            patch: spectrum_imaging::AdjustmentPatch {
                flip_horizontal: Some(true),
                ..Default::default()
            },
        },
    )
    .unwrap();
    let flipped = render_layer_preview(&document.layers[0], None)
        .unwrap()
        .into_rgba8();
    assert_eq!(
        flipped.pixels().map(|pixel| pixel[3]).collect::<Vec<_>>(),
        [0, 64]
    );
    assert_eq!(
        document.layers[0]
            .pixel_mask
            .as_ref()
            .unwrap()
            .alpha
            .as_ref(),
        [64, 0]
    );
    crate::command_apply::apply_command(
        &mut document,
        Command::AdjustLayer {
            id: 1,
            patch: spectrum_imaging::AdjustmentPatch {
                exposure: Some(0.5),
                rotation: Some(90),
                flip_horizontal: Some(false),
                crop: Some(Some(spectrum_imaging::CropRect {
                    x: 0.0,
                    y: 0.0,
                    width: 1.0,
                    height: 0.5,
                })),
                ..Default::default()
            },
        },
    )
    .unwrap();
    assert_eq!(document.layers[0].adjustments.exposure, 0.5);
    assert_eq!(
        document.layers[0]
            .pixel_mask
            .as_ref()
            .unwrap()
            .alpha
            .as_ref(),
        [64, 0]
    );
    let cropped = render_layer_preview(&document.layers[0], None).unwrap();
    assert_eq!((cropped.width(), cropped.height()), (1, 1));
    let full = render_document(&document, None).unwrap().into_rgba8();
    let region = render_document_region_scaled(
        &document,
        1.0,
        RenderRegion {
            x: 0,
            y: 0,
            width: document.width,
            height: document.height,
        },
    )
    .unwrap()
    .into_rgba8();
    assert_eq!(region, full);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn transparent_raster_mask_matches_worker_full_region_export_and_bounded_crop_geometry() {
    let directory = test_directory("transparent-worker-parity");
    std::fs::create_dir_all(&directory).unwrap();
    let source = directory.join("transparent.png");
    let source_pixels = image::RgbaImage::from_fn(12, 9, |x, y| {
        Rgba([
            (x * 29 + y * 7) as u8,
            (y * 37 + x * 3) as u8,
            (x * 11 + y * 19) as u8,
            ((x * 31 + y * 43) % 256) as u8,
        ])
    });
    source_pixels.save(&source).unwrap();
    let layer = Layer {
        id: 1,
        pixel_mask: Some(PixelMask::new(
            12,
            9,
            (0..108)
                .map(|index| ((index * 53 + 17) % 256) as u8)
                .collect::<Vec<_>>(),
        )),
        adjustments: spectrum_imaging::Adjustments {
            exposure: 0.35,
            rotation: 90,
            flip_horizontal: true,
            straighten: 9.0,
            crop: Some(spectrum_imaging::CropRect {
                x: 0.125,
                y: 0.0,
                width: 0.5,
                height: 1.0,
            }),
            ..Default::default()
        },
        transform: Transform {
            x: 6.0,
            y: 5.0,
            scale_x: 1.25,
            scale_y: 1.1,
            rotation: 17.0,
        },
        kind: LayerKind::Raster {
            path: source.clone(),
            original_path: None,
        },
        ..Layer::default()
    };

    let cached_larger_base = render_layer_base(&layer, Some(8)).unwrap();
    let unsafe_two_stage =
        render_layer_preview_from_base(&layer, cached_larger_base, Some(4)).unwrap();
    let core_preview = render_layer_preview(&layer, Some(4)).unwrap();
    assert_ne!(
        unsafe_two_stage, core_preview,
        "a genuinely downsampled larger cache entry is not exact and must not be reused for masked rasters"
    );

    let bounded_base = render_layer_base(&layer, Some(4)).unwrap();
    assert_eq!(bounded_base.dimensions(), (4, 3));
    let worker_equivalent = render_layer_preview_from_base(&layer, bounded_base, Some(4)).unwrap();
    assert_eq!(worker_equivalent, core_preview);
    let cached_valid_base = render_layer_base(&layer, Some(8)).unwrap();
    let mut malformed = layer.clone();
    malformed.pixel_mask = Some(PixelMask::new(11, 9, vec![255; 99]));
    assert!(
        render_layer_preview_from_base(&malformed, cached_valid_base, Some(4)).is_err(),
        "cached unmasked bases must still reject source-dimension mask mismatches"
    );

    let mut old_order = source_pixels.clone();
    crate::pixel_masks::apply_pixel_mask_region(
        &mut old_order,
        layer.pixel_mask.as_ref(),
        (12, 9),
        (0, 0),
    )
    .unwrap();
    let old_order = image::DynamicImage::ImageRgba8(old_order).resize(
        4,
        4,
        image::imageops::FilterType::Triangle,
    );
    let old_order = spectrum_imaging::render_image(
        old_order,
        layer.adjustments.clone(),
        spectrum_imaging::RenderOptions::default(),
    );
    assert_ne!(
        old_order, worker_equivalent,
        "transparent fixture must distinguish mask-before-geometry from the shared mask-after-geometry pipeline"
    );

    let mut document = Document::new("Transparent parity", 32, 28);
    document.background = [0; 4];
    document.layers.push(layer);
    document.selected = Some(1);
    document.next_id = 2;
    let full = render_document(&document, None).unwrap().into_rgba8();
    let region = render_document_region_scaled(
        &document,
        1.0,
        RenderRegion {
            x: 0,
            y: 0,
            width: document.width,
            height: document.height,
        },
    )
    .unwrap()
    .into_rgba8();
    assert_eq!(region, full);

    let export = directory.join("transparent-parity.png");
    export_document(&document, &export, 92).unwrap();
    assert_eq!(image::open(&export).unwrap().into_rgba8(), full);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn delete_selected_pixels_fails_atomically_for_empty_unsupported_and_locked_targets() {
    let directory = test_directory("errors");
    std::fs::create_dir_all(&directory).unwrap();
    let source = directory.join("opaque.png");
    image::RgbaImage::from_pixel(4, 4, Rgba([20, 30, 40, 255]))
        .save(&source)
        .unwrap();
    let mut workspace = Workspace::new(Document::new("Errors", 20, 20), None);
    workspace
        .execute(Command::AddRaster {
            path: source,
            name: None,
            x: 2.0,
            y: 3.0,
        })
        .unwrap();
    let before = workspace.document.clone();
    assert!(
        workspace
            .execute(Command::DeleteSelectedPixels { id: 1 })
            .is_err()
    );
    assert_eq!(workspace.document, before);

    workspace
        .execute(Command::SetSelection {
            selection: Some(Selection::rectangle(15, 15, 2, 2)),
        })
        .unwrap();
    let before = workspace.document.clone();
    assert!(
        workspace
            .execute(Command::DeleteSelectedPixels { id: 1 })
            .is_err()
    );
    assert_eq!(workspace.document, before);

    workspace.document.layer_mut(1).unwrap().locked = true;
    workspace.document.selection = Some(Selection::rectangle(2, 3, 2, 2));
    let before = workspace.document.clone();
    assert!(
        workspace
            .execute(Command::DeleteSelectedPixels { id: 1 })
            .is_err()
    );
    assert_eq!(workspace.document, before);

    workspace.document.layer_mut(1).unwrap().locked = false;
    workspace.document.layers.push(Layer {
        id: 2,
        kind: LayerKind::Rectangle {
            width: 4,
            height: 4,
            color: [255; 4],
            corner_radius: 0.0,
        },
        ..Default::default()
    });
    let before = workspace.document.clone();
    assert!(
        workspace
            .execute(Command::DeleteSelectedPixels { id: 2 })
            .is_err()
    );
    assert_eq!(workspace.document, before);
    std::fs::remove_dir_all(directory).unwrap();
}
