use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use image::{Rgba, RgbaImage};
use spectrum_imaging::{
    ExactRegionSource, PixelRegion, RegionReadCapability, RegionReadiness, RegionSourceDescriptor,
    RegionSourceInfo, SourceSampleDepth,
};

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

struct GradientMemorySource {
    info: RegionSourceInfo,
    pixels: RgbaImage,
    reads: Arc<AtomicUsize>,
}

impl ExactRegionSource for GradientMemorySource {
    type Error = std::convert::Infallible;

    fn info(&self) -> &RegionSourceInfo {
        &self.info
    }

    fn read_exact_region(&self, region: PixelRegion) -> Result<RgbaImage, Self::Error> {
        self.reads.fetch_add(1, Ordering::Relaxed);
        Ok(image::imageops::crop_imm(
            &self.pixels,
            region.x,
            region.y,
            region.width,
            region.height,
        )
        .to_image())
    }
}

struct GradientMemoryResolver {
    path: std::path::PathBuf,
    source: ResolvedRasterSource,
}

impl RasterSourceResolver for GradientMemoryResolver {
    fn snapshot_epoch(&self) -> u64 {
        1
    }

    fn resolve(&self, path: &std::path::Path) -> Option<ResolvedRasterSource> {
        (path == self.path).then(|| self.source.clone())
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
            GradientStop::new(1.0, [0, 0, 0, 0]),
        ],
        center: [0.43, 0.57],
        radius: 0.46,
        spread,
        interpolation: GradientInterpolation::PremultipliedSrgbV1,
        offset: 0.0,
        extent: 1.0,
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

fn metric_gradient(kind: GradientKind) -> ShapeGradient {
    ShapeGradient {
        kind,
        stops: vec![
            GradientStop::new(0.0, [0, 0, 0, 255]),
            GradientStop::new(1.0, [255, 255, 255, 255]),
        ],
        center: [0.5025, 0.505],
        radius: 0.8,
        ..Default::default()
    }
}

fn metric_layers(gradient: ShapeGradient) -> [Layer; 3] {
    let rectangle = Layer {
        id: 1,
        shape_fill: Some(ShapeFill::Gradient(gradient.clone())),
        kind: LayerKind::Rectangle {
            width: 200,
            height: 100,
            color: [255; 4],
            corner_radius: 0.0,
        },
        ..Default::default()
    };
    let ellipse = Layer {
        id: 2,
        shape_fill: Some(ShapeFill::Gradient(gradient.clone())),
        kind: LayerKind::Ellipse {
            width: 200,
            height: 100,
            color: [255; 4],
        },
        ..Default::default()
    };
    let path = Layer {
        id: 3,
        shape_fill: Some(ShapeFill::Gradient(gradient)),
        kind: LayerKind::Path {
            geometry: PathGeometry::new(
                200,
                100,
                true,
                PathFillRule::EvenOdd,
                vec![
                    PathAnchor::corner(0.0, 0.0),
                    PathAnchor::corner(200.0, 0.0),
                    PathAnchor::corner(200.0, 100.0),
                    PathAnchor::corner(0.0, 100.0),
                ],
            )
            .unwrap(),
            color: [255; 4],
        },
        ..Default::default()
    };
    [rectangle, ellipse, path]
}

#[test]
fn nonsquare_rectangle_ellipse_and_path_share_one_source_pixel_metric() {
    for layer in metric_layers(metric_gradient(GradientKind::Radial)) {
        let rendered = render_layer_preview(&layer, None).unwrap().to_rgba8();
        assert_eq!(
            rendered.get_pixel(140, 50),
            rendered.get_pixel(100, 90),
            "equal 40 px radial distances diverged for layer {}",
            layer.id
        );
    }
    for layer in metric_layers(metric_gradient(GradientKind::Angle)) {
        let rendered = render_layer_preview(&layer, None).unwrap().to_rgba8();
        assert_eq!(
            rendered.get_pixel(140, 90).0,
            [32, 32, 32, 255],
            "physical 45-degree angle diverged for layer {}",
            layer.id
        );
        assert_eq!(
            rendered.get_pixel(100, 10).0,
            [191, 191, 191, 255],
            "negative-y angle diverged for layer {}",
            layer.id
        );
    }

    let radial = ShapeFill::Gradient(metric_gradient(GradientKind::Radial));
    let sampler = radial.sampler(200, 100);
    assert_eq!(sampler.sample(150.5, 50.5), sampler.sample(100.5, 0.5));
    let angle = ShapeFill::Gradient(metric_gradient(GradientKind::Angle));
    let sampler = angle.sampler(200, 100);
    assert_eq!(sampler.sample(150.5, 50.5), [0, 0, 0, 255]);
    assert_eq!(sampler.sample(100.5, 0.5), [191, 191, 191, 255]);
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
fn gradient_regions_match_with_a_real_exact_region_provider() {
    let mut document = gradient_document();
    let pixels = RgbaImage::from_fn(document.width, document.height, |x, y| {
        Rgba([
            ((x * 17 + y * 3) % 256) as u8,
            ((x * 5 + y * 29) % 256) as u8,
            ((x * 31 + y * 7) % 256) as u8,
            255,
        ])
    });
    let path = std::env::temp_dir().join(format!(
        "prism-gradient-provider-{}-{}.png",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    pixels.save(&path).unwrap();
    document.layers.insert(
        0,
        Layer {
            id: 4,
            name: "Provider-backed raster".into(),
            kind: LayerKind::Raster {
                path: path.clone(),
                original_path: None,
            },
            ..Default::default()
        },
    );
    document.next_id = 5;

    let reads = Arc::new(AtomicUsize::new(0));
    let source = GradientMemorySource {
        info: RegionSourceInfo {
            descriptor: RegionSourceDescriptor {
                width: pixels.width(),
                height: pixels.height(),
                color_encoding: "rgba8".into(),
                sample_depth: SourceSampleDepth::EightBit,
                frame_index: 0,
                page_index: 0,
                decoder_contract: "prism-gradient-test-memory:v1".into(),
            },
            capability: RegionReadCapability::DerivedBacking,
            readiness: RegionReadiness::Ready,
        },
        pixels,
        reads: Arc::clone(&reads),
    };
    let resolver = GradientMemoryResolver {
        path: path.clone(),
        source: ResolvedRasterSource::new(
            RasterSourceEpoch::new("gradient-provider:v1").unwrap(),
            Arc::new(source),
        )
        .unwrap(),
    };

    let scale = 1.75;
    let full = render_document_scaled(&document, scale).unwrap().to_rgba8();
    for region in [
        RenderRegion {
            x: 7,
            y: 11,
            width: 91,
            height: 57,
        },
        RenderRegion {
            x: 103,
            y: 19,
            width: 73,
            height: 109,
        },
    ] {
        let oracle =
            image::imageops::crop_imm(&full, region.x, region.y, region.width, region.height)
                .to_image();
        assert_eq!(
            render_document_region_scaled_with_sources(&document, scale, region, &resolver)
                .unwrap()
                .to_rgba8(),
            oracle
        );
    }
    assert!(
        reads.load(Ordering::Relaxed) > 0,
        "the ExactRegionSource provider was not exercised"
    );
    std::fs::remove_file(path).unwrap();
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
