use std::{fs, sync::Arc, time::SystemTime};

use spectrum_revisions::{Actor, ActorKind, SessionId};

use crate::*;

fn sample(x: f32, y: f32) -> BrushSample {
    BrushSample {
        x,
        y,
        pressure: 1.0,
    }
}

fn stroke(samples: Vec<BrushSample>) -> BrushStroke {
    BrushStroke::new(BrushStyle::default(), samples).unwrap()
}

fn paint_layer(id: u64, program: BrushProgram) -> Layer {
    Layer {
        id,
        name: "Paint".into(),
        kind: LayerKind::Paint { program },
        ..Layer::default()
    }
}

fn sample_heavy_program() -> BrushProgram {
    let samples = vec![sample(1.5, 1.5); MAX_BRUSH_SAMPLES_PER_STROKE];
    let heavy = stroke(samples);
    let mut program = BrushProgram::new(32, 32).unwrap();
    for _ in 0..17 {
        program = program.append(heavy.clone()).unwrap();
    }
    program
}

#[test]
fn serde_rejects_overlong_sample_and_stroke_sequences() {
    let samples = vec![sample(1.5, 1.5); MAX_BRUSH_SAMPLES_PER_STROKE + 1];
    let stroke_json = serde_json::json!({
        "style": BrushStyle::default(),
        "samples": samples,
    });
    assert!(
        serde_json::from_value::<BrushStroke>(stroke_json)
            .unwrap_err()
            .to_string()
            .contains("sample count")
    );

    let one = serde_json::to_value(stroke(vec![sample(1.5, 1.5)])).unwrap();
    let program_json = serde_json::json!({
        "version": BRUSH_PROGRAM_VERSION,
        "width": 32,
        "height": 32,
        "strokes": vec![one; MAX_BRUSH_STROKES_PER_LAYER + 1],
    });
    assert!(
        serde_json::from_value::<BrushProgram>(program_json)
            .unwrap_err()
            .to_string()
            .contains("stroke count")
    );
}

#[test]
fn program_rejects_aggregate_sample_dab_and_clip_overflow() {
    let heavy = stroke(vec![sample(1.5, 1.5); MAX_BRUSH_SAMPLES_PER_STROKE]);
    let mut samples = BrushProgram::new(32, 32).unwrap();
    for _ in 0..MAX_BRUSH_SAMPLES_PER_DOCUMENT / MAX_BRUSH_SAMPLES_PER_STROKE {
        samples = samples.append(heavy.clone()).unwrap();
    }
    assert!(
        samples
            .append(heavy)
            .unwrap_err()
            .to_string()
            .contains("sample")
    );

    let dab_heavy = BrushStroke::new(
        BrushStyle {
            size: 1.0,
            spacing: 0.01,
            ..BrushStyle::default()
        },
        vec![sample(0.5, 0.5), sample(16_383.5, 0.5)],
    )
    .unwrap();
    let mut dabs = BrushProgram::new(16_384, 1).unwrap();
    for _ in 0..8 {
        dabs = dabs.append(dab_heavy.clone()).unwrap();
    }
    assert!(
        dabs.append(dab_heavy)
            .unwrap_err()
            .to_string()
            .contains("dab")
    );

    let alpha: Arc<[u8]> = vec![255; 1024 * 1024].into();
    let clipped = stroke(vec![sample(1.5, 1.5)])
        .with_clip(
            Some(BrushClip::Alpha {
                x: 0,
                y: 0,
                width: 1024,
                height: 1024,
                alpha,
            }),
            (1024, 1024),
        )
        .unwrap();
    let mut clips = BrushProgram::new(1024, 1024).unwrap();
    for _ in 0..16 {
        clips = clips.append(clipped.clone()).unwrap();
    }
    assert!(
        clips
            .append(clipped)
            .unwrap_err()
            .to_string()
            .contains("clip-byte")
    );
}

#[test]
fn duplicate_and_insert_validate_document_paint_budgets_atomically() {
    let program = sample_heavy_program();
    let mut source = Document::new("Source", 32, 32);
    source.layers.push(paint_layer(1, program.clone()));
    source.selected = Some(1);
    source.next_id = 2;
    let transfer = LayerTransfer::from_document(&source, 1).unwrap();

    let mut duplicate = Workspace::new(source.clone(), None);
    let before = duplicate.document.clone();
    assert!(
        duplicate
            .execute(Command::DuplicateLayer { id: 1 })
            .is_err()
    );
    assert_eq!(duplicate.document, before);

    let mut destination = Workspace::new(source, None);
    let before = destination.document.clone();
    assert!(
        destination
            .execute(Command::InsertLayer {
                transfer: Box::new(transfer),
                index: None,
            })
            .is_err()
    );
    assert_eq!(destination.document, before);
}

#[test]
fn transformed_selection_clip_is_frozen_when_selection_and_transform_change() {
    let program = BrushProgram::new(64, 64).unwrap();
    let mut document = Document::new("Frozen", 160, 120);
    document.layers.push(Layer {
        id: 1,
        transform: Transform {
            x: 30.0,
            y: 20.0,
            scale_x: 1.5,
            scale_y: 0.75,
            rotation: 18.0,
        },
        kind: LayerKind::Paint { program },
        ..Layer::default()
    });
    document.selected = Some(1);
    document.next_id = 2;
    document.selection = Some(Selection::rectangle(50, 35, 30, 24));
    let mut workspace = Workspace::new(document, None);
    workspace
        .execute(Command::AddBrushStroke {
            id: 1,
            stroke: stroke(vec![sample(5.5, 20.5), sample(55.5, 20.5)]),
            selection: PaintSelection::Current,
        })
        .unwrap();
    let LayerKind::Paint { program } = &workspace.document.layer(1).unwrap().kind else {
        panic!("expected Paint")
    };
    let identity = program.strokes[0].identity();

    workspace.document.selection = Some(Selection::rectangle(0, 0, 2, 2));
    workspace.document.layer_mut(1).unwrap().transform = Transform::default();
    let LayerKind::Paint { program } = &workspace.document.layer(1).unwrap().kind else {
        panic!("expected Paint")
    };
    assert_eq!(program.strokes[0].identity(), identity);
    assert!(program.strokes[0].clip.is_some());
}

#[test]
fn geometric_adjustments_fail_closed_before_mutating_paint() {
    for adjustments in [
        spectrum_imaging::Adjustments {
            rotation: 90,
            ..Default::default()
        },
        spectrum_imaging::Adjustments {
            flip_horizontal: true,
            ..Default::default()
        },
        spectrum_imaging::Adjustments {
            straighten: 2.0,
            ..Default::default()
        },
        spectrum_imaging::Adjustments {
            crop: Some(spectrum_imaging::CropRect {
                x: 0.1,
                y: 0.1,
                width: 0.8,
                height: 0.8,
            }),
            ..Default::default()
        },
    ] {
        let mut document = Document::new("Adjusted", 64, 64);
        document.layers.push(Layer {
            id: 1,
            adjustments,
            kind: LayerKind::Paint {
                program: BrushProgram::new(64, 32).unwrap(),
            },
            ..Layer::default()
        });
        document.selected = Some(1);
        document.next_id = 2;
        let mut workspace = Workspace::new(document, None);
        let before = workspace.document.clone();
        let error = workspace
            .execute(Command::AddBrushStroke {
                id: 1,
                stroke: stroke(vec![sample(4.5, 4.5)]),
                selection: PaintSelection::Current,
            })
            .unwrap_err();
        assert!(error.to_string().contains("geometric adjustments"));
        assert_eq!(workspace.document, before);
    }

    let mut document = Document::new("Exposure", 64, 64);
    document.layers.push(Layer {
        id: 1,
        adjustments: spectrum_imaging::Adjustments {
            exposure: 1.0,
            straighten: 0.01,
            ..Default::default()
        },
        kind: LayerKind::Paint {
            program: BrushProgram::new(64, 64).unwrap(),
        },
        ..Layer::default()
    });
    document.next_id = 2;
    assert!(
        Workspace::new(document, None)
            .execute(Command::AddBrushStroke {
                id: 1,
                stroke: stroke(vec![sample(4.5, 4.5)]),
                selection: PaintSelection::None,
            })
            .is_ok()
    );
}

#[test]
fn paint_gesture_is_durable_undoable_redoable_and_reopens_exactly() {
    let stamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let project = std::env::temp_dir().join(format!("prism-paint-history-{stamp}.prism"));
    let actor = Actor {
        id: "person:paint-test".into(),
        display_name: "Paint test".into(),
        kind: ActorKind::Human,
    };
    let session = SessionId::new();
    let mut workspace = Workspace::create_durable(
        Document::new("Paint history", 64, 64),
        &project,
        actor.clone(),
        session,
    )
    .unwrap();
    workspace
        .execute(Command::AddPaintLayerWithStroke {
            name: None,
            width: 64,
            height: 64,
            stroke: stroke(vec![sample(4.5, 4.5), sample(50.5, 40.5)]),
            selection: PaintSelection::None,
        })
        .unwrap();
    let expected = workspace.document.clone();
    workspace.execute(Command::Undo).unwrap();
    assert!(workspace.document.layers.is_empty());
    workspace.execute(Command::Redo).unwrap();
    assert_eq!(workspace.document, expected);
    workspace.save(None).unwrap();
    drop(workspace);

    let reopened = Workspace::open_as(&project, actor, session).unwrap();
    assert_eq!(reopened.document, expected);
    drop(reopened);
    fs::remove_file(project).unwrap();
}

#[test]
fn painting_never_mutates_or_converts_a_raster_original() {
    let stamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let original = std::env::temp_dir().join(format!("prism-paint-original-{stamp}.png"));
    image::RgbaImage::from_pixel(4, 4, image::Rgba([10, 20, 30, 255]))
        .save(&original)
        .unwrap();
    let before = fs::read(&original).unwrap();
    let mut document = Document::new("Original", 4, 4);
    document.layers.push(Layer {
        id: 1,
        kind: LayerKind::Raster {
            path: original.clone(),
            original_path: Some(original.clone()),
        },
        ..Layer::default()
    });
    document.selected = Some(1);
    document.next_id = 2;
    let mut workspace = Workspace::new(document, None);
    assert!(
        workspace
            .execute(Command::AddBrushStroke {
                id: 1,
                stroke: stroke(vec![sample(1.5, 1.5)]),
                selection: PaintSelection::None,
            })
            .is_err()
    );
    assert_eq!(fs::read(&original).unwrap(), before);
    assert!(matches!(
        workspace.document.layer(1).unwrap().kind,
        LayerKind::Raster { .. }
    ));
    fs::remove_file(original).unwrap();
}
