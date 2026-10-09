//! How much storage work each thing the desktop does while editing costs.
//!
//! Durability flushes, document opens, and re-reading large photos are cheap
//! on some machines and slow on others (macOS flushes the whole drive), so
//! timing one machine misses them. These budgets count the work instead, so
//! an edit, an idle check, or a read that starts doing more fails here on
//! every platform.
use spectrum_assets::Service;
use spectrum_document::{IoStats, hashed_bytes, io_stats};

/// Storage work allowed for one operation.
struct Budget {
    live_opens: u64,
    syncs: u64,
    publications: u64,
    hashed: u64,
}

fn check<T>(name: &str, budget: Budget, run: impl FnOnce() -> T) -> T {
    let (before, hashed_before) = (io_stats(), hashed_bytes());
    let value = run();
    let work: IoStats = io_stats().since(before);
    let hashed = hashed_bytes() - hashed_before;
    let within = work.live_opens <= budget.live_opens
        && work.syncs <= budget.syncs
        && work.publications <= budget.publications
        && hashed <= budget.hashed;
    assert!(
        within,
        "{name} did {work:?} and hashed {hashed} bytes; allowed live_opens {}, syncs {}, \
         publications {}, hashed {}",
        budget.live_opens, budget.syncs, budget.publications, budget.hashed
    );
    value
}

const READ: Budget = Budget {
    live_opens: 2,
    syncs: 0,
    publications: 0,
    hashed: 0,
};

const SAVE: Budget = Budget {
    live_opens: 1,
    syncs: 6,
    publications: 1,
    hashed: 0,
};

/// One test, so no other test's storage work lands in the counts.
#[test]
fn editing_an_image_and_a_canvas_stays_within_its_storage_budgets() {
    let tmp = tempfile::tempdir().unwrap();
    let source = tmp.path().join("photo.jpg");
    image::RgbImage::from_fn(1600, 1200, |x, y| {
        image::Rgb([(x % 251) as u8, (y % 241) as u8, 90])
    })
    .save(&source)
    .unwrap();
    let root = tmp.path().join("library");
    let mut service = Service::open(&root).unwrap();
    let image = service.import(vec![source]).unwrap().pop().unwrap();
    let canvas = service.create_canvas("Canvas".into(), 800, 600).unwrap();
    service.place(canvas.id, image.id).unwrap();

    // Opening reads the document; its photo is checked once per process.
    let (_, mut revision) = service.image_view(image.id).unwrap();
    Service::open(&root)
        .unwrap()
        .canvas_view(canvas.id)
        .unwrap();
    check("reopening an image", READ, || {
        service.image_view(image.id).unwrap()
    });
    let (_, mut canvas_revision) = check("reopening a canvas", READ, || {
        Service::open(&root)
            .unwrap()
            .canvas_view(canvas.id)
            .unwrap()
    });

    for step in 1..=3 {
        revision = check("saving an image edit", SAVE, || {
            let adjustments = spectrum_image::Adjustments {
                exposure: step as f32 * 0.1,
                ..Default::default()
            };
            Service::open(&root)
                .unwrap()
                .edit_image_from(
                    image.id,
                    Some(revision),
                    vec![spectrum_image::Command::SetAdjustments { adjustments }],
                )
                .unwrap()
                .1
        });
        canvas_revision = check("saving a canvas edit", SAVE, || {
            Service::open(&root)
                .unwrap()
                .edit_canvas_from(
                    canvas.id,
                    Some(canvas_revision),
                    vec![spectrum_canvas::Command::SetOpacity {
                        id: 1,
                        opacity: 1.0 - step as f32 * 0.1,
                    }],
                )
                .unwrap()
                .1
        });
    }

    // Checking for an agent's work while idle writes nothing.
    for _ in 0..3 {
        let followed = check("an idle follow check", READ, || {
            Service::open(&root).unwrap().follow(image.id).unwrap()
        });
        assert_eq!(followed, revision);
    }
    check("reading an image", READ, || {
        Service::open(&root).unwrap().image(image.id).unwrap()
    });
    check("reading a canvas", READ, || {
        Service::open(&root)
            .unwrap()
            .saved_canvas(canvas.id)
            .unwrap()
    });
}
