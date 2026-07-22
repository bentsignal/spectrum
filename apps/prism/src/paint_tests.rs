use image::imageops;

use crate::*;

fn sample(x: f32, y: f32) -> BrushSample {
    BrushSample {
        x,
        y,
        pressure: 1.0,
    }
}

fn stroke(mode: BrushMode, color: [u8; 4], opacity: f32, samples: Vec<BrushSample>) -> BrushStroke {
    BrushStroke::new(
        BrushStyle {
            mode,
            color,
            size: 12.0,
            hardness: 1.0,
            opacity,
            spacing: 0.25,
        },
        samples,
    )
    .unwrap()
}

fn paint_layer(program: BrushProgram) -> Layer {
    Layer {
        id: 1,
        name: "Paint".into(),
        kind: LayerKind::Paint { program },
        ..Layer::default()
    }
}

#[test]
fn paint_erase_repaint_preserves_rgb_and_canonicalizes_transparency() {
    let center = vec![sample(16.5, 16.5)];
    let program = BrushProgram::new(32, 32)
        .unwrap()
        .append(stroke(
            BrushMode::Paint,
            [220, 20, 10, 255],
            1.0,
            center.clone(),
        ))
        .unwrap()
        .append(stroke(BrushMode::Erase, [0; 4], 0.5, center.clone()))
        .unwrap();
    let partly_erased = crate::paint::render_paint_region(&program, None, 0, 0, 32, 32).unwrap();
    let pixel = partly_erased.get_pixel(16, 16).0;
    assert_eq!(&pixel[..3], &[220, 20, 10]);
    assert!(pixel[3] > 0 && pixel[3] < 255);

    let repainted = program
        .append(stroke(
            BrushMode::Paint,
            [5, 210, 30, 255],
            1.0,
            center.clone(),
        ))
        .unwrap();
    assert_eq!(
        crate::paint::render_paint_region(&repainted, None, 0, 0, 32, 32)
            .unwrap()
            .get_pixel(16, 16)
            .0,
        [5, 210, 30, 255]
    );
    let transparent = repainted
        .append(stroke(BrushMode::Erase, [0; 4], 1.0, center))
        .unwrap();
    assert_eq!(
        crate::paint::render_paint_region(&transparent, None, 0, 0, 32, 32)
            .unwrap()
            .get_pixel(16, 16)
            .0,
        [0; 4]
    );
}

#[test]
fn first_drag_is_one_undoable_command_and_selection_is_baked() {
    let mut workspace = Workspace::new(Document::new("Paint", 64, 64), None);
    workspace.document.selection = Some(Selection::rectangle(8, 8, 16, 16));
    workspace
        .execute(Command::AddPaintLayerWithStroke {
            name: None,
            width: 64,
            height: 64,
            stroke: stroke(
                BrushMode::Paint,
                [255; 4],
                1.0,
                vec![sample(4.5, 12.5), sample(30.5, 12.5)],
            ),
            selection: PaintSelection::Current,
        })
        .unwrap();
    let LayerKind::Paint { program } = &workspace.document.layer(1).unwrap().kind else {
        panic!("expected Paint layer")
    };
    let program = program.clone();
    assert!(matches!(
        program.strokes[0].clip,
        Some(BrushClip::Rectangle { .. })
    ));
    let rendered = crate::paint::render_paint_region(&program, None, 0, 0, 64, 64).unwrap();
    assert_eq!(rendered.get_pixel(4, 12)[3], 0);
    assert!(rendered.get_pixel(12, 12)[3] > 0);
    workspace.execute(Command::Undo).unwrap();
    assert!(workspace.document.layers.is_empty());
}

#[test]
fn malformed_snapshot_selection_is_rejected_atomically() {
    let mut workspace = Workspace::new(Document::new("Paint", 32, 32), None);
    let error = workspace
        .execute(Command::AddPaintLayerWithStroke {
            name: None,
            width: 32,
            height: 32,
            stroke: stroke(BrushMode::Paint, [255; 4], 1.0, vec![sample(8.5, 8.5)]),
            selection: PaintSelection::Snapshot {
                selection: Box::new(Selection::color_mask(0, 0, 2, 2, vec![255])),
            },
        })
        .unwrap_err();
    assert!(error.to_string().contains("alpha"));
    assert!(workspace.document.layers.is_empty());
}

#[test]
fn sixteen_k_diagonal_with_tiny_selection_captures_only_the_intersection() {
    let mut workspace = Workspace::new(Document::new("Paint", 16_384, 16_384), None);
    workspace.document.selection = Some(Selection::rectangle(8_000, 8_000, 8, 8));
    workspace
        .execute(Command::AddPaintLayerWithStroke {
            name: None,
            width: 16_384,
            height: 16_384,
            stroke: stroke(
                BrushMode::Paint,
                [255; 4],
                1.0,
                vec![sample(0.5, 0.5), sample(16_383.5, 16_383.5)],
            ),
            selection: PaintSelection::Current,
        })
        .unwrap();
    let LayerKind::Paint { program } = &workspace.document.layer(1).unwrap().kind else {
        panic!("expected Paint layer")
    };
    let program = program.clone();
    let clip = program.strokes[0].clip.as_ref().unwrap();
    let (width, height) = match clip {
        BrushClip::Rectangle { width, height, .. } | BrushClip::Alpha { width, height, .. } => {
            (*width, *height)
        }
    };
    assert!(width <= 8 && height <= 8);

    workspace.document.selection = Some(Selection::rectangle(0, 0, 2, 2));
    let rendered = crate::paint::render_paint_region(&program, None, 7_990, 7_990, 32, 32).unwrap();
    assert!(rendered.pixels().any(|pixel| pixel[3] > 0));
    assert_eq!(rendered.get_pixel(0, 0)[3], 0);
}

#[test]
fn sixteen_k_diagonal_with_select_all_uses_a_zero_byte_rectangle_clip() {
    let mut workspace = Workspace::new(Document::new("Paint", 16_384, 16_384), None);
    workspace.document.selection = Some(Selection::rectangle(0, 0, 16_384, 16_384));
    workspace
        .execute(Command::AddPaintLayerWithStroke {
            name: None,
            width: 16_384,
            height: 16_384,
            stroke: stroke(
                BrushMode::Paint,
                [255; 4],
                1.0,
                vec![sample(0.5, 0.5), sample(16_383.5, 16_383.5)],
            ),
            selection: PaintSelection::Current,
        })
        .unwrap();
    let LayerKind::Paint { program } = &workspace.document.layer(1).unwrap().kind else {
        panic!("expected Paint layer")
    };
    assert!(matches!(
        program.strokes[0].clip,
        Some(BrushClip::Rectangle {
            x: 0,
            y: 0,
            width: 16_384,
            height: 16_384
        })
    ));
    assert_eq!(program.clip_bytes(), 0);
}

#[test]
fn identity_is_stable_across_serde_and_append_invalidates_it() {
    let program = BrushProgram::new(64, 64)
        .unwrap()
        .append(stroke(
            BrushMode::Paint,
            [1, 2, 3, 255],
            0.75,
            vec![sample(4.5, 5.5), sample(50.5, 30.5)],
        ))
        .unwrap();
    assert_eq!(program, program.clone());
    let decoded: BrushProgram =
        serde_json::from_slice(&serde_json::to_vec(&program).unwrap()).unwrap();
    assert_eq!(decoded.identity(), program.identity());
    assert_eq!(decoded, program);
    let appended = program
        .append(stroke(
            BrushMode::Erase,
            [0; 4],
            1.0,
            vec![sample(10.5, 10.5)],
        ))
        .unwrap();
    assert_ne!(appended.identity(), program.identity());
}

#[test]
fn sixteen_k_preview_is_downsampled_before_full_source_allocation() {
    let program = BrushProgram::new(16_384, 16_384)
        .unwrap()
        .append(stroke(
            BrushMode::Paint,
            [255, 80, 30, 255],
            1.0,
            vec![sample(8_192.5, 8_192.5)],
        ))
        .unwrap();
    let preview = render_layer_base(&paint_layer(program), Some(256)).unwrap();
    assert_eq!((preview.width(), preview.height()), (256, 256));
}

#[test]
fn rotated_nonuniform_full_and_region_renders_are_exact() {
    let program = BrushProgram::new(64, 48)
        .unwrap()
        .append(stroke(
            BrushMode::Paint,
            [30, 180, 240, 220],
            0.9,
            vec![sample(6.5, 8.5), sample(55.5, 38.5)],
        ))
        .unwrap();
    let mut layer = paint_layer(program);
    layer.transform = Transform {
        x: 42.0,
        y: 38.0,
        scale_x: 1.4,
        scale_y: 0.7,
        rotation: 27.0,
    };
    let mut document = Document::new("Parity", 180, 140);
    document.layers.push(layer);
    let full = render_document(&document, None).unwrap().to_rgba8();
    let region = RenderRegion {
        x: 36,
        y: 30,
        width: 96,
        height: 78,
    };
    let cropped = render_document_region_scaled(&document, 1.0, region)
        .unwrap()
        .to_rgba8();
    assert_eq!(
        cropped,
        imageops::crop_imm(&full, region.x, region.y, region.width, region.height).to_image()
    );
}

#[test]
fn path_transfer_remains_v4_and_paint_v5_round_trips() {
    let geometry = PathGeometry::new(
        32,
        24,
        false,
        PathFillRule::EvenOdd,
        vec![PathAnchor::corner(2.0, 2.0), PathAnchor::corner(28.0, 20.0)],
    )
    .unwrap();
    let mut document = Document::new("Transfers", 64, 64);
    document.layers.push(Layer {
        id: 1,
        kind: LayerKind::Path {
            geometry,
            color: [255; 4],
        },
        ..Layer::default()
    });
    assert_eq!(
        LayerTransfer::from_document(&document, 1).unwrap().version,
        4
    );

    let program = BrushProgram::new(64, 64)
        .unwrap()
        .append(stroke(
            BrushMode::Paint,
            [255; 4],
            1.0,
            vec![sample(20.5, 20.5)],
        ))
        .unwrap();
    let mut paint = paint_layer(program);
    paint.id = 2;
    document.layers.push(paint);
    let transfer = LayerTransfer::from_document(&document, 2).unwrap();
    assert_eq!(transfer.version, 5);
    assert_eq!(
        LayerTransfer::from_json(&transfer.to_json().unwrap()).unwrap(),
        transfer
    );
}

#[test]
fn pressure_controls_diameter_and_coverage() {
    let style = BrushStyle {
        size: 20.0,
        hardness: 1.0,
        ..BrushStyle::default()
    };
    let full = BrushStroke::new(
        style,
        vec![BrushSample {
            x: 16.5,
            y: 16.5,
            pressure: 1.0,
        }],
    )
    .unwrap();
    let light = BrushStroke::new(
        style,
        vec![BrushSample {
            x: 16.5,
            y: 16.5,
            pressure: 0.5,
        }],
    )
    .unwrap();
    let full = crate::paint::render_paint_region(
        &BrushProgram::new(32, 32).unwrap().append(full).unwrap(),
        None,
        0,
        0,
        32,
        32,
    )
    .unwrap();
    let light = crate::paint::render_paint_region(
        &BrushProgram::new(32, 32).unwrap().append(light).unwrap(),
        None,
        0,
        0,
        32,
        32,
    )
    .unwrap();
    assert_eq!(full.get_pixel(16, 16)[3], 255);
    assert!((126..=128).contains(&light.get_pixel(16, 16)[3]));
    assert!(full.get_pixel(23, 16)[3] > 0);
    assert_eq!(light.get_pixel(23, 16)[3], 0);
}

#[test]
fn hardness_antialiases_edges_and_overlap_does_not_exceed_stroke_opacity() {
    let render = |hardness, samples| {
        let stroke = BrushStroke::new(
            BrushStyle {
                size: 16.0,
                hardness,
                opacity: 0.5,
                spacing: 0.05,
                ..BrushStyle::default()
            },
            samples,
        )
        .unwrap();
        crate::paint::render_paint_region(
            &BrushProgram::new(40, 24).unwrap().append(stroke).unwrap(),
            None,
            0,
            0,
            40,
            24,
        )
        .unwrap()
    };
    let soft = render(0.0, vec![sample(12.5, 12.5)]);
    let hard = render(1.0, vec![sample(12.5, 12.5)]);
    assert!(soft.get_pixel(18, 12)[3] < hard.get_pixel(18, 12)[3]);
    assert!(soft.get_pixel(18, 12)[3] > 0);

    let overlapping = render(1.0, vec![sample(8.5, 12.5), sample(24.5, 12.5)]);
    assert!(overlapping.pixels().all(|pixel| pixel[3] <= 128));
    assert_eq!(overlapping.get_pixel(16, 12)[3], 128);
}

#[test]
fn raw_paint_regions_match_arbitrary_tile_edge_crops() {
    let program = BrushProgram::new(140, 96)
        .unwrap()
        .append(stroke(
            BrushMode::Paint,
            [80, 190, 220, 210],
            0.8,
            vec![sample(3.5, 6.5), sample(136.5, 90.5)],
        ))
        .unwrap();
    let full = crate::paint::render_paint_region(&program, None, 0, 0, 140, 96).unwrap();
    let region = crate::paint::render_paint_region(&program, None, 61, 31, 67, 49).unwrap();
    assert_eq!(region, imageops::crop_imm(&full, 61, 31, 67, 49).to_image());
}

#[test]
fn transformed_paint_matches_across_output_compositor_tile_seams() {
    let program = BrushProgram::new(620, 240)
        .unwrap()
        .append(stroke(
            BrushMode::Paint,
            [210, 70, 160, 230],
            0.85,
            vec![sample(5.5, 20.5), sample(610.5, 220.5)],
        ))
        .unwrap();
    let mut layer = paint_layer(program);
    layer.transform = Transform {
        x: 24.0,
        y: 18.0,
        scale_x: 1.05,
        scale_y: 0.9,
        rotation: 7.0,
    };
    layer.blend_mode = BlendMode::Screen;
    let mut document = Document::new("Output seam", 700, 320);
    document.layers.push(layer);
    let full = render_document(&document, None).unwrap().to_rgba8();
    let region = RenderRegion {
        x: 480,
        y: 40,
        width: 96,
        height: 220,
    };
    let tile = render_document_region_scaled(&document, 1.0, region)
        .unwrap()
        .to_rgba8();
    assert_eq!(
        tile,
        imageops::crop_imm(&full, region.x, region.y, region.width, region.height).to_image()
    );
}

#[test]
fn paint_mask_adjustment_and_vector_mask_order_never_resurrects_transparency() {
    let program = BrushProgram::new(32, 32)
        .unwrap()
        .append(
            BrushStroke::new(
                BrushStyle {
                    size: 32.0,
                    hardness: 1.0,
                    ..BrushStyle::default()
                },
                vec![sample(16.0, 16.0)],
            )
            .unwrap(),
        )
        .unwrap();
    let path = PathGeometry::new(
        32,
        32,
        true,
        PathFillRule::EvenOdd,
        vec![
            PathAnchor::corner(0.0, 0.0),
            PathAnchor::corner(32.0, 0.0),
            PathAnchor::corner(32.0, 16.0),
            PathAnchor::corner(0.0, 16.0),
        ],
    )
    .unwrap();
    let mut alpha = vec![0; 32 * 32];
    for row in alpha.chunks_exact_mut(32) {
        row[..16].fill(255);
    }
    let mut layer = paint_layer(program);
    layer.pixel_mask = Some(PixelMask::new(32, 32, alpha));
    layer.adjustments.exposure = 2.0;
    layer.vector_mask = Some(VectorMask::new(path, false).unwrap());
    let mut document = Document::new("Mask order", 32, 32);
    document.background = [0; 4];
    document.layers.push(layer);
    let rendered = render_document(&document, None).unwrap().to_rgba8();
    assert!(rendered.get_pixel(12, 12)[3] > 0);
    assert_eq!(rendered.get_pixel(20, 12)[3], 0);
    assert_eq!(rendered.get_pixel(12, 20)[3], 0);
}

#[test]
fn adjusted_vector_masked_sixteen_k_paint_region_stays_bounded() {
    let program = BrushProgram::new(16_384, 16_384)
        .unwrap()
        .append(stroke(
            BrushMode::Paint,
            [255, 80, 30, 255],
            1.0,
            vec![sample(8_192.5, 8_192.5)],
        ))
        .unwrap();
    let mask_path = PathGeometry::new(
        16_384,
        16_384,
        true,
        PathFillRule::EvenOdd,
        vec![
            PathAnchor::corner(0.0, 0.0),
            PathAnchor::corner(16_384.0, 0.0),
            PathAnchor::corner(16_384.0, 16_384.0),
            PathAnchor::corner(0.0, 16_384.0),
        ],
    )
    .unwrap();
    let mut layer = paint_layer(program);
    layer.adjustments.rotation = 90;
    layer.adjustments.exposure = 0.5;
    layer.vector_mask = Some(VectorMask::new(mask_path, false).unwrap());
    let mut document = Document::new("Bounded Paint", 16_384, 16_384);
    document.layers.push(layer);
    let (rendered, stats) = render_document_region_scaled_with_stats(
        &document,
        1.0,
        RenderRegion {
            x: 8_064,
            y: 8_064,
            width: 256,
            height: 256,
        },
    )
    .unwrap();
    assert_eq!((rendered.width(), rendered.height()), (256, 256));
    assert!(stats.max_source_staging_pixels <= MAX_PAINT_REGION_PIXELS);
    assert!(stats.max_adjusted_staging_pixels <= MAX_PAINT_REGION_PIXELS);
    assert_eq!(stats.transformed_surface_pixels, 0);
}
