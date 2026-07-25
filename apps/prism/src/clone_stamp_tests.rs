use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use image::{Rgba, RgbaImage};
use spectrum_imaging::AdjustmentPatch;
use spectrum_imaging::PixelRegion;

use crate::*;

fn test_directory(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    fs::canonicalize(std::env::temp_dir())
        .unwrap_or_else(|_| std::env::temp_dir())
        .join(format!("prism-clone-{label}-{stamp}"))
}

fn write_source(path: &Path) {
    let mut image = RgbaImage::new(8, 8);
    for y in 0..8 {
        for x in 0..8 {
            image.put_pixel(
                x,
                y,
                Rgba([
                    20 + x as u8 * 20,
                    30 + y as u8 * 18,
                    200 - x as u8 * 10,
                    255,
                ]),
            );
        }
    }
    image.put_pixel(1, 1, Rgba([200, 50, 10, 128]));
    image.save(path).unwrap();
}

fn source_document(path: &Path) -> Document {
    let mut alpha = vec![255; 64];
    alpha[9] = 128;
    let mut document = Document::new("Clone", 8, 8);
    document.background = [0; 4];
    document.layers.push(Layer {
        id: 1,
        name: "Frozen source".into(),
        pixel_mask: Some(PixelMask::new(8, 8, alpha)),
        kind: LayerKind::Raster {
            path: path.to_owned(),
            original_path: None,
        },
        ..Layer::default()
    });
    document.selected = Some(1);
    document.next_id = 2;
    document
}

fn clone_stroke(samples: Vec<BrushSample>, size: f32) -> BrushStroke {
    BrushStroke::new(
        BrushStyle {
            mode: BrushMode::Paint,
            color: [0; 4],
            size,
            hardness: 1.0,
            opacity: 1.0,
            spacing: 0.25,
        },
        samples,
    )
    .unwrap()
    .as_current_clone()
    .unwrap()
}

fn sample(x: f32, y: f32) -> BrushSample {
    BrushSample {
        x,
        y,
        pressure: 1.0,
    }
}

fn paint_program(document: &Document, id: u64) -> &BrushProgram {
    let LayerKind::Paint { program } = &document.layer(id).unwrap().kind else {
        panic!("expected Paint layer")
    };
    program
}

#[test]
fn clone_stamp_applies_source_alpha_mask_and_destination_selection_once() {
    let directory = test_directory("alpha-selection");
    fs::create_dir_all(&directory).unwrap();
    let source = directory.join("source.png");
    write_source(&source);
    let mut workspace = Workspace::new(source_document(&source), None);
    workspace
        .execute(Command::SetCloneSource {
            id: 1,
            document_x: 1.5,
            document_y: 1.5,
            resolved_source: None,
        })
        .unwrap();
    workspace.document.selection = Some(Selection::rectangle(4, 4, 1, 1));
    let paint_id = workspace
        .execute(Command::AddPaintLayerWithStroke {
            name: Some("Clone".into()),
            width: 8,
            height: 8,
            stroke: clone_stroke(vec![sample(4.5, 4.5)], 3.0),
            selection: PaintSelection::Current,
        })
        .unwrap()
        .layer_ids[0];
    let rendered = crate::paint_render::render_paint_region(
        paint_program(&workspace.document, paint_id),
        None,
        0,
        0,
        8,
        8,
    )
    .unwrap();
    assert_eq!(rendered.get_pixel(4, 4).0, [200, 50, 10, 64]);
    assert_eq!(rendered.get_pixel(3, 4).0, [0; 4]);
    assert_eq!(rendered.get_pixel(5, 4).0, [0; 4]);
}

#[test]
fn clone_stamp_is_transparent_outside_source_and_never_feeds_back_destination_dabs() {
    let directory = test_directory("overlap");
    fs::create_dir_all(&directory).unwrap();
    let source = directory.join("source.png");
    write_source(&source);
    let mut workspace = Workspace::new(source_document(&source), None);
    workspace
        .execute(Command::SetCloneSource {
            id: 1,
            document_x: 0.5,
            document_y: 0.5,
            resolved_source: None,
        })
        .unwrap();
    let paint_id = workspace
        .execute(Command::AddPaintLayerWithStroke {
            name: None,
            width: 8,
            height: 8,
            stroke: clone_stroke(vec![sample(3.5, 3.5), sample(5.5, 3.5)], 3.0),
            selection: PaintSelection::None,
        })
        .unwrap()
        .layer_ids[0];
    let rendered = crate::paint_render::render_paint_region(
        paint_program(&workspace.document, paint_id),
        None,
        0,
        0,
        8,
        8,
    )
    .unwrap();
    assert_eq!(rendered.get_pixel(1, 3).0, [0; 4]);
    assert_eq!(rendered.get_pixel(3, 3).0, [20, 30, 200, 255]);
    assert_eq!(rendered.get_pixel(4, 3).0, [40, 30, 190, 255]);
    assert_eq!(rendered.get_pixel(5, 3).0, [60, 30, 180, 255]);
}

#[test]
fn clone_source_inverse_maps_transforms_and_rejects_unsupported_inputs() {
    let directory = test_directory("capture");
    fs::create_dir_all(&directory).unwrap();
    let source = directory.join("source.png");
    write_source(&source);
    let mut document = source_document(&source);
    document.layers[0].transform = Transform {
        x: 10.0,
        y: 20.0,
        scale_x: 2.0,
        scale_y: 3.0,
        rotation: 90.0,
    };
    let local = [2.5_f32, 1.5_f32];
    let document_point = local_to_document(local, (8, 8), document.layers[0].transform);
    let mut workspace = Workspace::new(document, None);
    workspace
        .execute(Command::SetCloneSource {
            id: 1,
            document_x: document_point[0],
            document_y: document_point[1],
            resolved_source: None,
        })
        .unwrap();
    let anchor = workspace
        .document
        .clone_source
        .as_ref()
        .unwrap()
        .anchor_local;
    assert!((anchor[0] - local[0]).abs() < 0.001);
    assert!((anchor[1] - local[1]).abs() < 0.001);

    workspace.document.layers[0].adjustments.rotation = 90;
    assert!(
        workspace
            .execute(Command::SetCloneSource {
                id: 1,
                document_x: document_point[0],
                document_y: document_point[1],
                resolved_source: None,
            })
            .unwrap_err()
            .to_string()
            .contains("crop, flip, rotation, or straighten")
    );
    workspace.document.layers.push(Layer {
        id: 2,
        name: "Paint is not a source".into(),
        kind: LayerKind::Paint {
            program: BrushProgram::new(8, 8).unwrap(),
        },
        ..Layer::default()
    });
    assert!(
        workspace
            .execute(Command::SetCloneSource {
                id: 2,
                document_x: 1.0,
                document_y: 1.0,
                resolved_source: None,
            })
            .unwrap_err()
            .to_string()
            .contains("must be a Raster layer")
    );
}

#[test]
fn durable_clone_survives_source_mutation_deletion_undo_redo_reopen_and_transfer() {
    let directory = test_directory("durable");
    fs::create_dir_all(&directory).unwrap();
    let source = directory.join("source.png");
    let project = directory.join("clone.prism");
    write_source(&source);
    let actor = spectrum_revisions::Actor {
        id: "person:clone-test".into(),
        display_name: "Clone test".into(),
        kind: spectrum_revisions::ActorKind::Human,
    };
    let mut workspace = Workspace::create_durable(
        source_document(&source),
        &project,
        actor,
        spectrum_revisions::SessionId::new(),
    )
    .unwrap();
    workspace
        .execute(Command::SetCloneSource {
            id: 1,
            document_x: 1.5,
            document_y: 1.5,
            resolved_source: None,
        })
        .unwrap();
    let paint_id = workspace
        .execute(Command::AddPaintLayerWithStroke {
            name: None,
            width: 8,
            height: 8,
            stroke: clone_stroke(vec![sample(4.5, 4.5)], 1.0),
            selection: PaintSelection::None,
        })
        .unwrap()
        .layer_ids[0];
    let baseline = crate::paint_render::render_paint_region(
        paint_program(&workspace.document, paint_id),
        None,
        0,
        0,
        8,
        8,
    )
    .unwrap();
    workspace
        .execute(Command::MoveLayer { id: 1, index: 1 })
        .unwrap();
    workspace
        .execute(Command::AdjustLayer {
            id: 1,
            patch: AdjustmentPatch {
                exposure: Some(2.0),
                ..AdjustmentPatch::default()
            },
        })
        .unwrap();
    workspace.execute(Command::RemoveLayer { id: 1 }).unwrap();
    fs::remove_file(&source).unwrap();
    let after = crate::paint_render::render_paint_region(
        paint_program(&workspace.document, paint_id),
        None,
        0,
        0,
        8,
        8,
    )
    .unwrap();
    assert_eq!(after, baseline);

    workspace.execute(Command::Undo).unwrap();
    workspace.execute(Command::Undo).unwrap();
    workspace.execute(Command::Undo).unwrap();
    assert_eq!(
        crate::paint_render::render_paint_region(
            paint_program(&workspace.document, paint_id),
            None,
            0,
            0,
            8,
            8,
        )
        .unwrap(),
        baseline
    );
    workspace.execute(Command::Redo).unwrap();
    workspace.execute(Command::Redo).unwrap();
    workspace.execute(Command::Redo).unwrap();
    drop(workspace);

    let reopened = Workspace::open(&project).unwrap();
    assert_eq!(
        crate::paint_render::render_paint_region(
            paint_program(&reopened.document, paint_id),
            None,
            0,
            0,
            8,
            8,
        )
        .unwrap(),
        baseline
    );
    let transfer = LayerTransfer::from_document(&reopened.document, paint_id).unwrap();
    assert_eq!(transfer.version, CLONE_STAMP_LAYER_TRANSFER_VERSION);
    let decoded = LayerTransfer::from_json(&transfer.to_json().unwrap()).unwrap();
    assert_eq!(decoded, transfer);
    let mut legacy = transfer.clone();
    legacy.version = PAINT_LAYER_TRANSFER_VERSION;
    assert!(legacy.to_json().is_err());
}

#[test]
fn sampled_source_matches_frozen_develop_pixel_mask_and_vector_mask_appearance() {
    let directory = test_directory("appearance");
    fs::create_dir_all(&directory).unwrap();
    let source = directory.join("source.png");
    write_source(&source);
    let mut document = source_document(&source);
    document.layers[0].adjustments.exposure = 0.75;
    document.layers[0].adjustments.contrast = 18.0;
    document.layers[0].vector_mask = Some(
        VectorMask::new(
            PathGeometry::new(
                8,
                8,
                true,
                PathFillRule::EvenOdd,
                vec![
                    PathAnchor::corner(0.0, 0.0),
                    PathAnchor::corner(8.0, 0.0),
                    PathAnchor::corner(0.0, 8.0),
                ],
            )
            .unwrap(),
            false,
        )
        .unwrap(),
    );
    let layer = document.layers[0].clone();
    let expected =
        render_layer_preview_from_base(&layer, render_layer_base(&layer, None).unwrap(), None)
            .unwrap()
            .to_rgba8();
    let snapshot = SampledSourceSnapshot::capture(&layer, [1.5, 1.5]).unwrap();
    let sampled = crate::sampled_source::sampled_source_region(
        &snapshot,
        PixelRegion {
            x: 0,
            y: 0,
            width: 8,
            height: 8,
        },
        None,
    )
    .unwrap();
    assert_eq!(sampled, expected);
}

#[test]
fn clone_source_hash_tampering_and_oversized_inputs_fail_atomically() {
    let directory = test_directory("tamper-limits");
    fs::create_dir_all(&directory).unwrap();
    let source = directory.join("source.png");
    write_source(&source);
    let mut workspace = Workspace::new(source_document(&source), None);
    workspace
        .execute(Command::SetCloneSource {
            id: 1,
            document_x: 1.5,
            document_y: 1.5,
            resolved_source: None,
        })
        .unwrap();
    let before = workspace.document.clone();
    RgbaImage::from_pixel(8, 8, Rgba([1, 2, 3, 255]))
        .save(&source)
        .unwrap();
    let error = workspace
        .execute(Command::AddPaintLayerWithStroke {
            name: None,
            width: 8,
            height: 8,
            stroke: clone_stroke(vec![sample(4.5, 4.5)], 1.0),
            selection: PaintSelection::None,
        })
        .unwrap_err();
    assert!(format!("{error:#}").contains("captured SHA-256"));
    assert_eq!(workspace.document, before);

    let oversized = directory.join("oversized.png");
    fs::File::create(&oversized)
        .unwrap()
        .set_len(crate::revisions::MAX_EMBEDDED_RASTER_BYTES as u64 + 1)
        .unwrap();
    let mut oversized_document = source_document(&oversized);
    let before = oversized_document.clone();
    let error = crate::apply_command(
        &mut oversized_document,
        Command::SetCloneSource {
            id: 1,
            document_x: 1.5,
            document_y: 1.5,
            resolved_source: None,
        },
    )
    .unwrap_err();
    assert!(format!("{error:#}").contains("512 MiB"));
    assert_eq!(oversized_document, before);
}

#[test]
fn durable_repeated_clone_strokes_deduplicate_the_captured_asset() {
    let directory = test_directory("dedup");
    fs::create_dir_all(&directory).unwrap();
    let source = directory.join("source.png");
    let project = directory.join("dedup.prism");
    write_source(&source);
    let actor = spectrum_revisions::Actor {
        id: "person:clone-dedup".into(),
        display_name: "Clone dedup".into(),
        kind: spectrum_revisions::ActorKind::Human,
    };
    let mut workspace = Workspace::create_durable(
        source_document(&source),
        &project,
        actor,
        spectrum_revisions::SessionId::new(),
    )
    .unwrap();
    workspace
        .execute(Command::SetCloneSource {
            id: 1,
            document_x: 1.5,
            document_y: 1.5,
            resolved_source: None,
        })
        .unwrap();
    let paint_id = workspace
        .execute(Command::AddPaintLayer {
            name: None,
            width: 8,
            height: 8,
        })
        .unwrap()
        .layer_ids[0];
    workspace
        .execute_batch(vec![
            Command::AddBrushStroke {
                id: paint_id,
                stroke: clone_stroke(vec![sample(3.5, 3.5)], 1.0),
                selection: PaintSelection::None,
            },
            Command::AddBrushStroke {
                id: paint_id,
                stroke: clone_stroke(vec![sample(5.5, 5.5)], 1.0),
                selection: PaintSelection::None,
            },
        ])
        .unwrap();
    workspace
        .execute_batch(
            (0..100)
                .map(|index| Command::RenameDocument {
                    name: format!("Clone dedup {index}"),
                })
                .collect(),
        )
        .unwrap();
    let connection = rusqlite::Connection::open(&project).unwrap();
    let asset_count: i64 = connection
        .query_row("SELECT count(*) FROM assets", [], |row| row.get(0))
        .unwrap();
    assert_eq!(asset_count, 1);
    let clone_operation_versions = connection
        .prepare(
            "SELECT DISTINCT version FROM operation_payloads
             WHERE instr(CAST(bytes AS TEXT), 'set_clone_source') > 0
                OR instr(CAST(bytes AS TEXT), 'clone_stamp') > 0",
        )
        .unwrap()
        .query_map([], |row| row.get::<_, u32>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(clone_operation_versions, vec![13]);
    let v10_snapshots: u32 = connection
        .query_row(
            "SELECT count(*) FROM snapshots WHERE version = 10",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(v10_snapshots, 1);
    let distinct_references = paint_program(&workspace.document, paint_id)
        .strokes
        .iter()
        .map(|stroke| stroke.sampled_source().unwrap().path.clone())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(distinct_references.len(), 1);
}

#[test]
fn together_collaboration_materializes_clone_assets_for_the_follower() {
    let directory = test_directory("collaboration");
    fs::create_dir_all(&directory).unwrap();
    let source = directory.join("source.png");
    let project = directory.join("collaboration.prism");
    write_source(&source);
    let human_session = spectrum_revisions::SessionId::new();
    let mut human = Workspace::create_durable(
        source_document(&source),
        &project,
        spectrum_revisions::Actor {
            id: "person:clone-human".into(),
            display_name: "Clone human".into(),
            kind: spectrum_revisions::ActorKind::Human,
        },
        human_session,
    )
    .unwrap();
    let collaboration = Workspace::start_collaboration(
        &project,
        Some(human_session),
        spectrum_revisions::Actor {
            id: "agent:clone".into(),
            display_name: "Clone agent".into(),
            kind: spectrum_revisions::ActorKind::Agent,
        },
        spectrum_revisions::CollaborationMode::Together,
    )
    .unwrap();
    let mut agent = Workspace::open_session(&project, collaboration.agent_session).unwrap();
    agent
        .execute(Command::SetCloneSource {
            id: 1,
            document_x: 1.5,
            document_y: 1.5,
            resolved_source: None,
        })
        .unwrap();
    let paint_id = agent
        .execute(Command::AddPaintLayerWithStroke {
            name: None,
            width: 8,
            height: 8,
            stroke: clone_stroke(vec![sample(4.5, 4.5)], 1.0),
            selection: PaintSelection::None,
        })
        .unwrap()
        .layer_ids[0];
    assert!(matches!(
        human.sync_together().unwrap(),
        spectrum_revisions::CollaborationSync::Advanced { .. }
    ));
    let agent_pixels = crate::paint_render::render_paint_region(
        paint_program(&agent.document, paint_id),
        None,
        0,
        0,
        8,
        8,
    )
    .unwrap();
    let human_pixels = crate::paint_render::render_paint_region(
        paint_program(&human.document, paint_id),
        None,
        0,
        0,
        8,
        8,
    )
    .unwrap();
    assert_eq!(human_pixels, agent_pixels);
    assert_ne!(
        paint_program(&human.document, paint_id).strokes[0]
            .sampled_source()
            .unwrap()
            .path,
        source
    );
}

#[test]
fn optimized_copy_preserves_clone_history_and_exact_sampled_pixels() {
    const STATIC_FONT: &[u8] =
        include_bytes!("../../../crates/spectrum-fonts/tests/fonts/noto-sans-static-source.ttf");

    let directory = test_directory("optimized-copy");
    fs::create_dir_all(&directory).unwrap();
    let source_image = directory.join("source.png");
    let font_path = directory.join("NotoSans.ttf");
    let project = directory.join("source.prism");
    let optimized = directory.join("optimized.prism");
    write_source(&source_image);
    let mut reducible_font = STATIC_FONT.to_vec();
    reducible_font.resize(256 * 1024, 0);
    fs::write(&font_path, reducible_font).unwrap();
    let mut workspace = Workspace::create_durable(
        Document::new("Clone optimized copy", 16, 16),
        &project,
        spectrum_revisions::Actor {
            id: "person:clone-optimized-copy".into(),
            display_name: "Clone optimized copy".into(),
            kind: spectrum_revisions::ActorKind::Human,
        },
        spectrum_revisions::SessionId::new(),
    )
    .unwrap();
    workspace
        .execute(Command::ImportFont {
            path: font_path,
            source_name: None,
        })
        .unwrap();
    let font_id = workspace.document.font_assets[0].id;
    let text_id = workspace.document.next_id;
    let source_id = text_id + 1;
    workspace
        .execute_batch(vec![
            Command::AddText {
                text: "A".into(),
                name: None,
                font_size: 12.0,
                color: [255; 4],
                x: 0.0,
                y: 0.0,
            },
            Command::SetTextTypography {
                id: text_id,
                typography: TextTypography {
                    font_id: Some(font_id),
                    ..TextTypography::default()
                },
            },
            Command::AddRaster {
                path: source_image.clone(),
                name: Some("Clone source".into()),
                x: 0.0,
                y: 0.0,
            },
            Command::SetCloneSource {
                id: source_id,
                document_x: 1.5,
                document_y: 1.5,
                resolved_source: None,
            },
            Command::AddPaintLayerWithStroke {
                name: Some("Clone result".into()),
                width: 16,
                height: 16,
                stroke: clone_stroke(vec![sample(8.5, 8.5)], 3.0),
                selection: PaintSelection::None,
            },
            Command::RemoveLayer { id: source_id },
        ])
        .unwrap();
    let paint_id = workspace
        .document
        .layers
        .iter()
        .find(|layer| layer.name == "Clone result")
        .unwrap()
        .id;
    let expected = crate::paint_render::render_paint_region(
        paint_program(&workspace.document, paint_id),
        None,
        0,
        0,
        16,
        16,
    )
    .unwrap();
    fs::remove_file(source_image).unwrap();
    drop(workspace);

    let report = create_optimized_font_copy(&project, &optimized).unwrap();
    assert!(report.output_bytes < report.source_bytes);
    let reopened = Workspace::open(&optimized).unwrap();
    let optimized_paint_id = reopened
        .document
        .layers
        .iter()
        .find(|layer| layer.name == "Clone result")
        .unwrap()
        .id;
    let actual = crate::paint_render::render_paint_region(
        paint_program(&reopened.document, optimized_paint_id),
        None,
        0,
        0,
        16,
        16,
    )
    .unwrap();
    assert_eq!(actual, expected);
}

fn local_to_document(local: [f32; 2], dimensions: (u32, u32), transform: Transform) -> [f32; 2] {
    let center = [
        dimensions.0 as f32 * transform.scale_x * 0.5,
        dimensions.1 as f32 * transform.scale_y * 0.5,
    ];
    let dx = local[0] * transform.scale_x - center[0];
    let dy = local[1] * transform.scale_y - center[1];
    let radians = transform.rotation.to_radians();
    let (sin, cos) = radians.sin_cos();
    [
        transform.x + center[0] + dx * cos - dy * sin,
        transform.y + center[1] + dx * sin + dy * cos,
    ]
}
