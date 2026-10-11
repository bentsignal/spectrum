//! An agent editing while the person's app reads and moves through the same
//! image's history, as when the person watches the agent work. Neither may
//! ever see an error from the other's writes.
use spectrum_assets::Service;
use spectrum_document::CollaborationMode;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

#[test]
fn reading_and_moving_while_an_agent_edits_never_fails() {
    let tmp = tempfile::tempdir().unwrap();
    let source = tmp.path().join("photo.png");
    image::RgbaImage::from_pixel(16, 16, image::Rgba([90, 90, 90, 255]))
        .save(&source)
        .unwrap();
    let root = tmp.path().join("library");
    let image = Service::open(&root).unwrap().import(vec![source]).unwrap()[0].id;
    let agent = Service::agent(&root).unwrap();
    let started = agent
        .start_agent(image, CollaborationMode::Separate, Some("Helper"))
        .unwrap();
    let session = started.agent_session;
    let done = Arc::new(AtomicBool::new(false));
    let writer = {
        let (root, done) = (root.clone(), done.clone());
        std::thread::spawn(move || {
            let agent = Service::agent(&root)
                .unwrap()
                .with_session(Some(session))
                .unwrap();
            for step in 0..120 {
                let adjustments = spectrum_image::Adjustments {
                    exposure: (step % 10) as f32 / 10.,
                    ..Default::default()
                };
                agent.set_adjustments(image, adjustments).unwrap();
            }
            done.store(true, Ordering::SeqCst);
        })
    };
    // The app: open, read the history, and follow the agent, again and again.
    let mut reads = 0;
    while !done.load(Ordering::SeqCst) {
        let mut person = Service::open(&root).unwrap();
        let history = person.history(image).unwrap();
        let place = person.session_place(image, session).unwrap();
        if place != history.current {
            person.move_to(image, place).unwrap();
        }
        reads += 1;
    }
    writer.join().unwrap();
    assert!(reads > 0);
}
