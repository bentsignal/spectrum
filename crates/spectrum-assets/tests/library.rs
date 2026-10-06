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
    // An agent edits the image together with the person, who sees it once
    // they follow; their canvases then show it too.
    Service::agent(&root)
        .unwrap()
        .adjust(
            image.id,
            serde_json::from_str("{\"exposure\":1.0}").unwrap(),
        )
        .unwrap();
    drop(service);
    let service = Service::open(&root).unwrap();
    assert_eq!(service.preview(image.id).unwrap(), before);
    service.follow(image.id).unwrap();
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

#[test]
fn agents_branch_from_the_person_and_never_overwrite_their_work() {
    let tmp = tempfile::tempdir().unwrap();
    let source = tmp.path().join("photo.png");
    image::RgbaImage::from_pixel(8, 8, image::Rgba([64, 64, 64, 255]))
        .save(&source)
        .unwrap();
    let root = tmp.path().join("library");
    let mut person = Service::open(&root).unwrap();
    let image = person.import(vec![source]).unwrap().pop().unwrap();
    let exposure = |value: f32| serde_json::from_str(&format!("{{\"exposure\":{value}}}")).unwrap();
    person.adjust(image.id, exposure(0.5)).unwrap();
    let (_, mine) = person.image_view(image.id).unwrap();

    // A separate agent works from where the person is, apart from them.
    let agent = Service::agent(&root).unwrap();
    let session = agent
        .start_agent(
            image.id,
            spectrum_document::CollaborationMode::Separate,
            Some("Helper"),
        )
        .unwrap()
        .agent_session;
    let helper = Service::agent(&root)
        .unwrap()
        .with_session(Some(session))
        .unwrap();
    assert_eq!(helper.image(image.id).unwrap().adjustments.exposure, 0.5);
    helper.adjust(image.id, exposure(2.0)).unwrap();
    assert_eq!(helper.image(image.id).unwrap().adjustments.exposure, 2.0);
    let (seen, at) = person.image_view(image.id).unwrap();
    assert_eq!((seen.adjustments.exposure, at), (0.5, mine));

    // The person keeps working; their edit and the agent's are siblings.
    let (_, next) = person
        .edit_image_from(
            image.id,
            Some(mine),
            vec![spectrum_image::Command::Adjust {
                patch: exposure(1.0),
            }],
        )
        .unwrap();
    let history = person.history(image.id).unwrap();
    assert_eq!(history.current, next);
    assert_eq!(
        history
            .revisions
            .iter()
            .filter(|revision| revision.parent_id == Some(mine))
            .count(),
        2
    );
    assert_eq!(helper.image(image.id).unwrap().adjustments.exposure, 2.0);

    // Jumping to the agent's revision and editing branches from it.
    let agent_at = helper.history(image.id).unwrap().current;
    person.move_to(image.id, agent_at).unwrap();
    assert_eq!(person.image(image.id).unwrap().adjustments.exposure, 2.0);
    person.adjust(image.id, exposure(3.0)).unwrap();
    assert_eq!(helper.image(image.id).unwrap().adjustments.exposure, 2.0);
    assert_eq!(person.image(image.id).unwrap().adjustments.exposure, 3.0);

    // By default an agent works together: the person follows it until they
    // edit, and the agent then starts again from the person's new work.
    let cli = Service::agent(&root).unwrap();
    cli.adjust(image.id, exposure(-1.0)).unwrap();
    assert_eq!(person.image(image.id).unwrap().adjustments.exposure, 3.0);
    person.follow(image.id).unwrap();
    assert_eq!(person.image(image.id).unwrap().adjustments.exposure, -1.0);
    person.adjust(image.id, exposure(0.25)).unwrap();
    cli.adjust(image.id, exposure(-0.5)).unwrap();
    assert_eq!(person.image(image.id).unwrap().adjustments.exposure, 0.25);
    person.follow(image.id).unwrap();
    let followed = person.image(image.id).unwrap();
    assert_eq!(followed.adjustments.exposure, -0.5);
    assert!(person.history(image.id).unwrap().sessions.len() >= 4);
}
