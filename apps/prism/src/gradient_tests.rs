use crate::*;

struct EmptyResolver;

impl RasterSourceResolver for EmptyResolver {
    fn snapshot_epoch(&self) -> u64 {
        1
    }

    fn resolve(&self, _path: &std::path::Path) -> Option<ResolvedRasterSource> {
        None
    }
}

fn modern_gradient(kind: GradientKind, spread: GradientSpread) -> ShapeGradient {
    ShapeGradient {
        kind,
        angle: 37.0,
        stops: vec![
            GradientStop::new(0.0, [245, 35, 70, 255]),
            GradientStop::new(0.23, [250, 210, 40, 190]),
            GradientStop::new(0.61, [25, 220, 180, 110]),
            GradientStop::new(1.0, [65, 45, 240, 0]),
        ],
        center: [0.43, 0.57],
        radius: 0.46,
        spread,
    }
}

fn closed_path() -> PathGeometry {
    PathGeometry::new(
        47,
        39,
        true,
        PathFillRule::EvenOdd,
        vec![
            PathAnchor::corner(2.0, 2.0),
            PathAnchor::corner(45.0, 5.0),
            PathAnchor::corner(38.0, 36.0),
            PathAnchor::corner(7.0, 32.0),
        ],
    )
    .unwrap()
}

fn gradient_document() -> Document {
    let mut document = Document::new("Modern gradients", 127, 91);
    document.background = [13, 21, 34, 117];
    document.layers = vec![
        Layer {
            id: 1,
            transform: Transform {
                x: 4.0,
                y: 3.0,
                rotation: 7.0,
                ..Default::default()
            },
            shape_fill: Some(ShapeFill::Gradient(modern_gradient(
                GradientKind::Linear,
                GradientSpread::Reflect,
            ))),
            kind: LayerKind::Rectangle {
                width: 58,
                height: 43,
                color: [255; 4],
                corner_radius: 6.0,
            },
            ..Default::default()
        },
        Layer {
            id: 2,
            opacity: 0.81,
            transform: Transform {
                x: 53.0,
                y: 12.0,
                scale_x: 1.11,
                scale_y: 0.93,
                rotation: -13.0,
            },
            shape_fill: Some(ShapeFill::Gradient(modern_gradient(
                GradientKind::Radial,
                GradientSpread::Repeat,
            ))),
            kind: LayerKind::Ellipse {
                width: 52,
                height: 47,
                color: [255; 4],
            },
            ..Default::default()
        },
        Layer {
            id: 3,
            opacity: 0.73,
            blend_mode: BlendMode::Screen,
            transform: Transform {
                x: 31.0,
                y: 45.0,
                rotation: 11.0,
                ..Default::default()
            },
            shape_fill: Some(ShapeFill::Gradient(modern_gradient(
                GradientKind::Angle,
                GradientSpread::Pad,
            ))),
            kind: LayerKind::Path {
                geometry: closed_path(),
                color: [255; 4],
            },
            ..Default::default()
        },
    ];
    document.next_id = 4;
    document
}

#[test]
fn modern_gradients_match_full_export_for_arbitrary_regions_and_uneven_strips() {
    let document = gradient_document();
    let export_path = std::env::temp_dir().join(format!(
        "prism-gradient-export-{}.png",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    export_document(&document, &export_path, 90).unwrap();
    assert_eq!(
        image::open(&export_path).unwrap().to_rgba8(),
        render_document_scaled(&document, 1.0).unwrap().to_rgba8()
    );
    std::fs::remove_file(export_path).unwrap();

    let scale = 1.75;
    let full = render_document_scaled(&document, scale).unwrap().to_rgba8();
    for region in [
        RenderRegion {
            x: 0,
            y: 0,
            width: 19,
            height: 27,
        },
        RenderRegion {
            x: 17,
            y: 9,
            width: 83,
            height: 61,
        },
        RenderRegion {
            x: 103,
            y: 37,
            width: 78,
            height: 91,
        },
    ] {
        let region_image = render_document_region_scaled(&document, scale, region)
            .unwrap()
            .to_rgba8();
        let oracle =
            image::imageops::crop_imm(&full, region.x, region.y, region.width, region.height)
                .to_image();
        assert_eq!(region_image, oracle);
    }

    let direct_region = RenderRegion {
        x: 11,
        y: 7,
        width: 117,
        height: 83,
    };
    let oracle = image::imageops::crop_imm(
        &full,
        direct_region.x,
        direct_region.y,
        direct_region.width,
        direct_region.height,
    )
    .to_image();
    assert_eq!(
        render_direct_preview_region_scaled(&document, scale, direct_region)
            .unwrap()
            .to_rgba8(),
        oracle
    );
    assert_eq!(
        render_direct_preview_region_scaled_with_sources(
            &document,
            scale,
            direct_region,
            &EmptyResolver,
        )
        .unwrap()
        .to_rgba8(),
        oracle
    );

    let mut x = 0;
    for width in [3, 17, 1, 41, 29, 73, 59] {
        if x >= full.width() {
            break;
        }
        let width = width.min(full.width() - x);
        let region = RenderRegion {
            x,
            y: 0,
            width,
            height: full.height(),
        };
        let strip = render_document_region_scaled(&document, scale, region)
            .unwrap()
            .to_rgba8();
        assert_eq!(
            strip,
            image::imageops::crop_imm(&full, x, 0, width, full.height()).to_image()
        );
        x += width;
    }
}

#[test]
fn malformed_and_oversized_gradients_never_mutate_the_document() {
    let mut workspace = Workspace::new(Document::new("Atomic", 64, 64), None);
    workspace
        .execute(Command::AddRectangle {
            name: None,
            width: 20,
            height: 20,
            color: [255; 4],
            corner_radius: 0.0,
            x: 0.0,
            y: 0.0,
        })
        .unwrap();
    let before = workspace.document.clone();
    let invalid = [
        ShapeGradient {
            stops: vec![
                GradientStop::new(0.5, [0; 4]),
                GradientStop::new(0.5, [255; 4]),
            ],
            ..Default::default()
        },
        ShapeGradient {
            center: [f32::NAN, 0.5],
            ..Default::default()
        },
        ShapeGradient {
            kind: GradientKind::Radial,
            spread: GradientSpread::Repeat,
            radius: f32::from_bits(1),
            ..Default::default()
        },
        ShapeGradient {
            kind: GradientKind::Radial,
            spread: GradientSpread::Reflect,
            radius: f32::MIN_POSITIVE / 2.0,
            ..Default::default()
        },
        ShapeGradient {
            angle: f32::INFINITY,
            ..Default::default()
        },
        ShapeGradient {
            stops: (0..33)
                .map(|index| GradientStop::new(index as f32 / 32.0, [index as u8, 0, 0, 255]))
                .collect(),
            ..Default::default()
        },
    ];
    for gradient in invalid {
        assert!(
            workspace
                .execute(Command::SetShapeFill {
                    id: 1,
                    fill: Some(ShapeFill::Gradient(gradient)),
                })
                .is_err()
        );
        assert_eq!(workspace.document, before);
    }
}

#[test]
fn one_fill_command_covers_rectangle_ellipse_and_closed_path() {
    let mut workspace = Workspace::new(gradient_document(), None);
    for (id, kind) in [
        (1, GradientKind::Linear),
        (2, GradientKind::Radial),
        (3, GradientKind::Angle),
    ] {
        let gradient = modern_gradient(kind, GradientSpread::Reflect);
        workspace
            .execute(Command::SetShapeFill {
                id,
                fill: Some(ShapeFill::Gradient(gradient.clone())),
            })
            .unwrap();
        assert_eq!(
            workspace.document.layer(id).unwrap().shape_fill,
            Some(ShapeFill::Gradient(gradient))
        );
    }
}

#[test]
fn v15_live_envelope_is_required_and_v14_fails_closed() {
    let command = Command::SetShapeFill {
        id: 1,
        fill: Some(ShapeFill::Gradient(modern_gradient(
            GradientKind::Radial,
            GradientSpread::Reflect,
        ))),
    };
    assert_eq!(
        required_command_operations_version(std::slice::from_ref(&command)),
        15
    );
    let expectation = PrismLiveActionExpectation {
        agent_revision: spectrum_revisions::RevisionId::new(),
        source_revision: None,
    };
    assert!(
        PrismLiveAction::ExecuteBatch {
            expectation: expectation.clone(),
            command_version: 14,
            commands: vec![command.clone()],
        }
        .validate()
        .is_err()
    );
    PrismLiveAction::ExecuteBatch {
        expectation,
        command_version: 15,
        commands: vec![command],
    }
    .validate()
    .unwrap();
}

#[test]
fn required_live_v15_emits_one_ordered_revision_and_together_reopens_exactly() {
    use spectrum_live_bridge::{BridgeEventKind, InteractionPolicy, ResponseBody};
    use spectrum_revisions::CollaborationMode;

    let mut fixture = crate::live_bridge_tests::Fixture::new(CollaborationMode::Together);
    let mut harness = crate::live_bridge_tests::HostHarness::new(&fixture);
    let subscription = harness.server.events().subscribe(0).unwrap();
    let gradient = modern_gradient(GradientKind::Radial, GradientSpread::Reflect);
    let request = harness.request(
        &fixture,
        PrismLiveAction::ExecuteBatch {
            expectation: fixture.expectation(),
            command_version: 15,
            commands: vec![
                Command::AddRectangle {
                    name: Some("Live gradient".into()),
                    width: 72,
                    height: 48,
                    color: [255; 4],
                    corner_radius: 0.0,
                    x: 4.0,
                    y: 5.0,
                },
                Command::SetShapeFill {
                    id: 1,
                    fill: Some(ShapeFill::Gradient(gradient.clone())),
                },
            ],
        },
        InteractionPolicy::Immediate,
    );
    let response = harness.round_trip(
        &mut fixture.human,
        request.clone(),
        PrismLiveInteractionState::Idle,
    );
    assert!(matches!(response.body, ResponseBody::Applied { .. }));
    let event = subscription.try_next().unwrap().unwrap();
    let revision_seq = event.seq;
    let BridgeEventKind::RevisionCommitted {
        request_id,
        session_id,
        cursors,
        ..
    } = event.event
    else {
        panic!("required-live gradient did not emit a revision event")
    };
    assert_eq!(request_id, Some(request.request_id));
    assert_eq!(session_id, fixture.agent_session);
    assert_eq!(
        cursors.len(),
        1,
        "one durable batch emits one cursor transition"
    );
    let collaboration = subscription.try_next().unwrap().unwrap();
    assert!(collaboration.seq > revision_seq);
    assert!(matches!(
        collaboration.event,
        BridgeEventKind::CollaborationAdvanced {
            agent_session_id,
            ..
        } if agent_session_id == fixture.agent_session
    ));
    assert!(subscription.try_next().unwrap().is_none());

    let agent = Workspace::open_session(&fixture.path, fixture.agent_session).unwrap();
    let fresh = Workspace::load_read_only(&fixture.path).unwrap();
    for document in [&fixture.human.document, &agent.document, &fresh] {
        assert_eq!(
            document.layer(1).unwrap().shape_fill,
            Some(ShapeFill::Gradient(gradient.clone()))
        );
    }
}

#[test]
fn modern_gradient_is_one_durable_revision_with_reopen_undo_redo() {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let project = std::env::temp_dir().join(format!("prism-gradient-{stamp}.prism"));
    let actor = spectrum_revisions::Actor {
        id: "person:gradient-test".into(),
        display_name: "Gradient test".into(),
        kind: spectrum_revisions::ActorKind::Human,
    };
    let session = spectrum_revisions::SessionId::new();
    let mut workspace = Workspace::create_durable(
        Document::new("Durable gradient", 80, 60),
        &project,
        actor.clone(),
        session,
    )
    .unwrap();
    workspace
        .execute(Command::AddRectangle {
            name: None,
            width: 40,
            height: 30,
            color: [255; 4],
            corner_radius: 0.0,
            x: 4.0,
            y: 5.0,
        })
        .unwrap();
    let gradient = modern_gradient(GradientKind::Angle, GradientSpread::Repeat);
    workspace
        .execute(Command::SetShapeFill {
            id: 1,
            fill: Some(ShapeFill::Gradient(gradient.clone())),
        })
        .unwrap();
    workspace.save(None).unwrap();
    drop(workspace);

    let connection = rusqlite::Connection::open(&project).unwrap();
    let v15: u32 = connection
        .query_row(
            "SELECT count(*) FROM operation_payloads WHERE version = 15",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(v15, 1);
    drop(connection);

    let mut reopened = Workspace::open_as(&project, actor, session).unwrap();
    assert_eq!(
        reopened.document.layer(1).unwrap().shape_fill,
        Some(ShapeFill::Gradient(gradient.clone()))
    );
    reopened.execute(Command::Undo).unwrap();
    assert!(reopened.document.layer(1).unwrap().shape_fill.is_none());
    reopened.execute(Command::Redo).unwrap();
    assert_eq!(
        reopened.document.layer(1).unwrap().shape_fill,
        Some(ShapeFill::Gradient(gradient))
    );
    drop(reopened);
    std::fs::remove_file(project).unwrap();
}
