use spectrum::library::{Service, actor};
use spectrum_revisions::SessionId;

#[test]
fn shared_images_survive_restart_and_deep_copies_are_independent() {
    let tmp = tempfile::tempdir().unwrap();
    let source = tmp.path().join("graphic.png");
    image::RgbaImage::from_pixel(8, 8, image::Rgba([64, 64, 64, 255]))
        .save(&source)
        .unwrap();
    let root = tmp.path().join("library");
    let mut service = Service::open(&root).unwrap();
    let image = service.import(vec![source.clone()]).unwrap().pop().unwrap();
    std::fs::remove_file(source).unwrap(); // Originals are owned by the app.
    let a = service.create_canvas("A".into(), 8, 8).unwrap();
    let b = service.create_canvas("B".into(), 8, 8).unwrap();
    service.place(a.id, image.id).unwrap();
    service.place(a.id, image.id).unwrap();
    service.place(b.id, image.id).unwrap();
    let copy = service.copy(a.id).unwrap();
    let mut copy_doc =
        prism_core::Workspace::load_read_only(&service.library.path(&copy).unwrap()).unwrap();
    let copied_image = copy_doc.layers[0].image_asset.unwrap();
    assert_ne!(copied_image, image.id);
    assert_eq!(copy_doc.layers[1].image_asset, Some(copied_image));
    let before = service.preview(image.id).unwrap();
    let mut workspace = lumen_core::Workspace::open_as(
        &service.library.path(&image).unwrap(),
        actor(),
        SessionId::new(),
    )
    .unwrap();
    workspace
        .execute(lumen_core::Command::Adjust {
            id: image.item.unwrap(),
            patch: serde_json::from_str("{\"exposure\":1.0}").unwrap(),
        })
        .unwrap();
    drop(workspace);
    drop(service);
    let mut service = Service::open(&root).unwrap();
    service.scan().unwrap();
    let after = service.preview(image.id).unwrap();
    assert_ne!(before, after);
    for canvas in [&a, &b] {
        let mut doc =
            prism_core::Workspace::load_read_only(&service.library.path(canvas).unwrap()).unwrap();
        service.resolve(&mut doc).unwrap();
        let rendered = prism_core::render_document(&doc, None).unwrap().to_rgba8();
        assert!(rendered.get_pixel(0, 0)[0] > 64);
    }
    service.resolve(&mut copy_doc).unwrap();
    let rendered = prism_core::render_document(&copy_doc, None)
        .unwrap()
        .to_rgba8();
    assert_eq!(rendered.get_pixel(0, 0)[0], 64);
    assert_eq!(
        service.image(copied_image).unwrap().adjustments.exposure,
        0.0
    );
    assert_eq!(service.library.dependents(image.id).unwrap().len(), 2);
}

#[test]
fn cli_adapter_undo_targets_the_requested_image_and_canvas() {
    let tmp = tempfile::tempdir().unwrap();
    let source = tmp.path().join("source.png");
    image::RgbaImage::from_pixel(8, 8, image::Rgba([64, 64, 64, 255]))
        .save(&source)
        .unwrap();
    let service = Service::open(&tmp.path().join("library")).unwrap();
    let mut assets = service.import(vec![source.clone()]).unwrap();
    // Add another image to the same internal document to exercise selection.
    let path = service.library.path(&assets[0]).unwrap();
    let mut workspace = lumen_core::Workspace::open_as(&path, actor(), SessionId::new()).unwrap();
    let source2 = tmp.path().join("second.png");
    std::fs::copy(&source, &source2).unwrap();
    workspace
        .execute(lumen_core::Command::Import {
            paths: vec![source2],
        })
        .unwrap();
    drop(workspace);
    assets = service.index_catalog(&path).unwrap();
    let second = assets.iter().find(|a| a.item == Some(2)).unwrap();
    spectrum::library::live::image(
        &path,
        2,
        lumen_core::Command::Adjust {
            id: 2,
            patch: serde_json::from_str("{\"exposure\":1.0}").unwrap(),
        },
    )
    .unwrap();
    spectrum::library::live::image(&path, 2, lumen_core::Command::Undo).unwrap();
    assert_eq!(service.image(second.id).unwrap().adjustments.exposure, 0.0);
    let canvas = service.create_canvas("Canvas".into(), 8, 8).unwrap();
    let path = service.library.path(&canvas).unwrap();
    spectrum::library::live::canvas(
        &path,
        vec![prism_core::Command::RenameDocument {
            name: "Renamed".into(),
        }],
    )
    .unwrap();
    spectrum::library::live::canvas(&path, vec![prism_core::Command::Undo]).unwrap();
    assert_eq!(
        prism_core::Workspace::load_read_only(&path).unwrap().name,
        "Canvas"
    );
}
