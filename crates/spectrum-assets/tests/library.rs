use spectrum_assets::Service;

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
    let mut copy_doc = service.saved_canvas(copy.id).unwrap();
    let copied_image = copy_doc.layers[0].image_asset.unwrap();
    assert_ne!(copied_image, image.id);
    assert_eq!(copy_doc.layers[1].image_asset, Some(copied_image));
    let before = service.preview(image.id).unwrap();
    // An agent edits the image; the person's canvases show it.
    Service::agent(&root)
        .unwrap()
        .adjust(
            image.id,
            serde_json::from_str("{\"exposure\":1.0}").unwrap(),
        )
        .unwrap();
    drop(service);
    let service = Service::open(&root).unwrap();
    let after = service.preview(image.id).unwrap();
    assert_ne!(before, after);
    for canvas in [&a, &b] {
        let doc = service.canvas(canvas.id).unwrap();
        let rendered = spectrum_canvas::render_document(&doc, None)
            .unwrap()
            .to_rgba8();
        assert!(rendered.get_pixel(0, 0)[0] > 64);
    }
    service.resolve(&mut copy_doc).unwrap();
    let rendered = spectrum_canvas::render_document(&copy_doc, None)
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
fn each_image_is_its_own_document_with_its_own_history() {
    let tmp = tempfile::tempdir().unwrap();
    let mut sources = Vec::new();
    for name in ["first.png", "second.png"] {
        let source = tmp.path().join(name);
        image::RgbaImage::from_pixel(8, 8, image::Rgba([64, 64, 64, 255]))
            .save(&source)
            .unwrap();
        sources.push(source);
    }
    let root = tmp.path().join("library");
    let mut service = Service::open(&root).unwrap();
    let assets = service.import(sources).unwrap();
    assert_eq!(assets[0].name, "first");
    assert_ne!(assets[0].document, assets[1].document);
    let exposure = serde_json::from_str("{\"exposure\":1.0}").unwrap();
    service.adjust(assets[1].id, exposure).unwrap();
    assert_eq!(
        service.image(assets[0].id).unwrap().adjustments.exposure,
        0.0
    );
    assert_eq!(
        service.image(assets[1].id).unwrap().adjustments.exposure,
        1.0
    );
    service.step_history(assets[1].id, false).unwrap();
    assert_eq!(
        service.image(assets[1].id).unwrap().adjustments.exposure,
        0.0
    );
    service.step_history(assets[1].id, true).unwrap();
    assert_eq!(
        service.image(assets[1].id).unwrap().adjustments.exposure,
        1.0
    );
    let renamed = service.rename(assets[0].id, " Cover ").unwrap();
    assert_eq!(renamed.name, "Cover");

    let canvas = service.create_canvas("Canvas".into(), 8, 8).unwrap();
    service
        .edit_canvas(
            canvas.id,
            vec![
                spectrum_canvas::Command::RenameDocument {
                    name: "Renamed".into(),
                },
                spectrum_canvas::Command::Undo,
            ],
        )
        .unwrap();
    assert_eq!(service.saved_canvas(canvas.id).unwrap().name, "Canvas");
    assert!(service.edit_canvas(assets[0].id, Vec::new()).is_err());

    // Purging removes the document and its cache.
    let document = root.join(&assets[0].document);
    service.delete(assets[0].id).unwrap();
    service.purge(assets[0].id).unwrap();
    assert!(!document.exists());
    assert_eq!(service.library.list().unwrap().len(), 2);
}

#[test]
fn export_writes_images_and_canvases_outside_the_library() {
    let tmp = tempfile::tempdir().unwrap();
    let source = tmp.path().join("source.png");
    image::RgbaImage::from_pixel(12, 8, image::Rgba([10, 200, 30, 255]))
        .save(&source)
        .unwrap();
    let mut service = Service::open(&tmp.path().join("library")).unwrap();
    let image = service.import(vec![source]).unwrap().pop().unwrap();
    let canvas = service.create_canvas("Poster".into(), 16, 16).unwrap();
    service.place(canvas.id, image.id).unwrap();
    let out = tmp.path().join("out");
    std::fs::create_dir_all(&out).unwrap();
    assert_eq!(service.export_size(image.id).unwrap(), (12, 8));
    assert_eq!(service.export_size(canvas.id).unwrap(), (16, 16));
    service.export(image.id, &out.join("image.jpg")).unwrap();
    service.export(canvas.id, &out.join("canvas.png")).unwrap();
    assert_eq!(
        image::image_dimensions(out.join("image.jpg")).unwrap(),
        (12, 8)
    );
    assert_eq!(
        image::image_dimensions(out.join("canvas.png")).unwrap(),
        (16, 16)
    );
    let inside = service.library.root().join("sneaky.png");
    assert!(service.export(image.id, &inside).is_err());
}
