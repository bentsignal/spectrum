use super::*;

fn square_document() -> Workspace {
    let mut document = Document::new("Erase", 80, 80);
    document.background = [0, 0, 0, 0];
    let mut workspace = Workspace::new(document, None);
    workspace
        .execute(Command::AddRectangle {
            name: None,
            width: 40,
            height: 40,
            color: [200, 40, 60, 255],
            corner_radius: 0.0,
            x: 10.0,
            y: 10.0,
        })
        .unwrap();
    workspace
}

fn alpha(document: &Document, x: u32, y: u32) -> u8 {
    render_document_scaled(document, 1.0).unwrap().to_rgba8()[(x, y)][3]
}

#[test]
fn hiding_a_selection_on_a_shape_clears_only_what_it_covers() {
    let mut workspace = square_document();
    workspace
        .execute(Command::SetSelection {
            selection: Some(Selection::rectangle(20, 20, 10, 10)),
        })
        .unwrap();
    workspace.execute(Command::HideSelection { id: 1 }).unwrap();
    let document = &workspace.document;
    assert_eq!(alpha(document, 25, 25), 0);
    assert_eq!(alpha(document, 15, 15), 255);
    assert_eq!(alpha(document, 45, 45), 255);
    // The region renderer the canvas draws with agrees with a full render.
    let full = render_document_scaled(document, 2.0).unwrap().to_rgba8();
    let region = render_document_region_scaled(
        document,
        2.0,
        RenderRegion {
            x: 0,
            y: 0,
            width: 160,
            height: 160,
        },
    )
    .unwrap();
    assert_eq!(full.as_raw(), region.to_rgba8().as_raw());
    workspace.execute(Command::Undo).unwrap();
    assert_eq!(alpha(&workspace.document, 25, 25), 255);
}

#[test]
fn hiding_where_a_layer_shows_nothing_is_refused() {
    let mut workspace = square_document();
    workspace
        .execute(Command::SetSelection {
            selection: Some(Selection::rectangle(60, 60, 10, 10)),
        })
        .unwrap();
    assert!(workspace.execute(Command::HideSelection { id: 1 }).is_err());
}

#[test]
fn the_hidden_part_follows_the_layer_when_it_turns() {
    let mut workspace = square_document();
    workspace
        .execute(Command::SetSelection {
            selection: Some(Selection::rectangle(10, 10, 20, 40)),
        })
        .unwrap();
    workspace.execute(Command::HideSelection { id: 1 }).unwrap();
    assert_eq!(alpha(&workspace.document, 15, 30), 0);
    assert_eq!(alpha(&workspace.document, 45, 30), 255);
    workspace
        .execute(Command::SetTransform {
            id: 1,
            transform: Transform {
                rotation: 180.0,
                ..workspace.document.layer(1).unwrap().transform
            },
        })
        .unwrap();
    // Turned half way, the hidden left half is now on the right.
    assert_eq!(alpha(&workspace.document, 15, 30), 255);
    assert_eq!(alpha(&workspace.document, 45, 30), 0);
}

#[test]
fn hiding_a_selection_on_text_keeps_the_text_editable() {
    let mut document = Document::new("Text", 400, 120);
    document.background = [0, 0, 0, 0];
    let mut workspace = Workspace::new(document, None);
    workspace
        .execute(Command::AddText {
            text: "HHHHHH".into(),
            name: None,
            font_size: 64.0,
            color: [255, 255, 255, 255],
            x: 10.0,
            y: 10.0,
            shaping: TextShaping::default(),
        })
        .unwrap();
    let id = workspace.document.selected.unwrap();
    let geometry =
        document_layer_geometry(&workspace.document, workspace.document.layer(id).unwrap())
            .unwrap();
    let middle = (geometry.center[0]) as u32;
    workspace
        .execute(Command::SetSelection {
            selection: Some(Selection::rectangle(0, 0, middle, 120)),
        })
        .unwrap();
    workspace.execute(Command::HideSelection { id }).unwrap();
    let rendered = render_document_scaled(&workspace.document, 1.0)
        .unwrap()
        .to_rgba8();
    let opaque = |columns: std::ops::Range<u32>| {
        columns
            .flat_map(|x| (0..120).map(move |y| (x, y)))
            .filter(|&(x, y)| rendered[(x, y)][3] > 0)
            .count()
    };
    assert_eq!(opaque(0..middle.saturating_sub(1)), 0);
    assert!(opaque(middle + 1..400) > 100);
    assert!(matches!(
        workspace.document.layer(id).unwrap().kind,
        LayerKind::Text { .. }
    ));
}

#[test]
fn erasing_a_stroke_from_a_shape_clears_under_the_brush() {
    let mut workspace = square_document();
    let style = BrushStyle {
        mode: BrushMode::Erase,
        size: 8.0,
        hardness: 1.0,
        ..BrushStyle::default()
    };
    let samples = [[12.0, 30.0], [48.0, 30.0]].map(|[x, y]| BrushSample {
        x,
        y,
        pressure: 1.0,
    });
    workspace
        .execute(Command::EraseLayer {
            id: 1,
            stroke: BrushStroke::new(style, samples.to_vec()).unwrap(),
        })
        .unwrap();
    assert_eq!(alpha(&workspace.document, 30, 30), 0);
    assert_eq!(alpha(&workspace.document, 30, 20), 255);
    // A stroke that misses the layer is refused rather than recorded.
    let away = [BrushSample {
        x: 70.0,
        y: 70.0,
        pressure: 1.0,
    }];
    assert!(
        workspace
            .execute(Command::EraseLayer {
                id: 1,
                stroke: BrushStroke::new(style, away.to_vec()).unwrap(),
            })
            .is_err()
    );
}

#[test]
fn painted_masks_survive_a_durable_round_trip() {
    let directory = std::env::temp_dir().join(format!(
        "prism-painted-mask-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("erase.prism");
    let session = spectrum_revisions::SessionId::new();
    let actor = spectrum_revisions::Actor {
        id: "person:erase".into(),
        display_name: "Eraser".into(),
        kind: spectrum_revisions::ActorKind::Human,
    };
    let mut document = Document::new("Erase", 80, 80);
    document.background = [0, 0, 0, 0];
    let mut workspace = Workspace::create_durable(document, &path, actor.clone(), session).unwrap();
    workspace
        .execute(Command::AddRectangle {
            name: None,
            width: 40,
            height: 40,
            color: [200, 40, 60, 255],
            corner_radius: 0.0,
            x: 10.0,
            y: 10.0,
        })
        .unwrap();
    workspace
        .execute(Command::SetSelection {
            selection: Some(Selection::rectangle(20, 20, 10, 10)),
        })
        .unwrap();
    workspace.execute(Command::HideSelection { id: 1 }).unwrap();
    let before = render_document_scaled(&workspace.document, 1.0)
        .unwrap()
        .to_rgba8();
    drop(workspace);
    let reopened = Workspace::open_as(&path, actor, session).unwrap();
    let after = render_document_scaled(&reopened.document, 1.0)
        .unwrap()
        .to_rgba8();
    assert_eq!(before.as_raw(), after.as_raw());
    drop(reopened);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn erased_images_render_the_same_from_the_interactive_cache() {
    let directory = std::env::temp_dir().join(format!(
        "prism-erase-image-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("photo.png");
    image::RgbaImage::from_fn(64, 48, |x, y| {
        image::Rgba([x as u8 * 4, y as u8 * 5, 90, 255])
    })
    .save(&path)
    .unwrap();
    let mut document = Document::new("Erase image", 80, 60);
    document.background = [0, 0, 0, 0];
    document.layers.push(Layer {
        id: 1,
        kind: LayerKind::Raster {
            path,
            original_path: None,
        },
        transform: Transform {
            x: 8.0,
            y: 6.0,
            scale_x: 0.75,
            scale_y: 0.75,
            rotation: 0.0,
        },
        ..Layer::default()
    });
    document.next_id = 2;
    let mut workspace = Workspace::new(document, None);
    let style = BrushStyle {
        mode: BrushMode::Erase,
        size: 10.0,
        hardness: 1.0,
        ..BrushStyle::default()
    };
    let samples = [[10.0, 25.0], [50.0, 25.0]].map(|[x, y]| BrushSample {
        x,
        y,
        pressure: 1.0,
    });
    workspace
        .execute(Command::EraseLayer {
            id: 1,
            stroke: BrushStroke::new(style, samples.to_vec()).unwrap(),
        })
        .unwrap();
    let document = &workspace.document;
    let sources = prepare_export_raster_sources(document, &directory.join("cache")).unwrap();
    let region = RenderRegion {
        x: 0,
        y: 0,
        width: 160,
        height: 120,
    };
    let render = || {
        render_document_region_scaled_with_sources(document, 2.0, region, &sources)
            .unwrap()
            .to_rgba8()
    };
    let _serial = crate::raster_backing_cache::cache_test_serial();
    let uncached = render();
    assert_eq!(uncached[(60, 50)][3], 0, "erased under the stroke");
    assert_eq!(uncached[(60, 20)][3], 255, "kept away from it");
    set_interactive_source_cache(true);
    let first = render();
    let again = render();
    set_interactive_source_cache(false);
    assert_eq!(uncached.as_raw(), first.as_raw());
    assert_eq!(uncached.as_raw(), again.as_raw());
    std::fs::remove_dir_all(directory).ok();
}
