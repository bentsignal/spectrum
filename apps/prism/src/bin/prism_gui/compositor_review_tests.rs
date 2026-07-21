use super::tests::{geometry, provider_snapshot};
use super::*;

fn detached_preview(receiver: Receiver<CompositeRenderResult>) -> CompositePreview {
    let senders = std::array::from_fn(|_| {
        let (sender, _receiver) = mpsc::channel();
        sender
    });
    CompositePreview {
        generation: 0,
        next_sequence: 1,
        desired: None,
        ready: None,
        failed: None,
        pending: None,
        workers_busy: [true, false],
        workers_available: [true, true],
        senders,
        receiver,
        active_this_frame: false,
    }
}

#[test]
fn reset_then_unconditional_poll_drains_late_result_and_snapshot() {
    let (sender, receiver) = mpsc::channel();
    let mut preview = detached_preview(receiver);
    let document = Document::new("Late result", 1, 1);
    let geometry = CanvasGeometry {
        canvas: Rect::from_min_size(Pos2::ZERO, Vec2::splat(1.0)),
        viewport: Rect::from_min_size(Pos2::ZERO, Vec2::splat(1.0)),
        pixels_per_point: 1.0,
    };
    let snapshot = Arc::new(RasterSourceSnapshot::empty());
    let weak_snapshot = Arc::downgrade(&snapshot);
    let key = CompositePreviewKey::new_with_sources(
        1,
        preview.generation,
        &document,
        geometry,
        1.0,
        snapshot.as_ref(),
    )
    .unwrap();
    preview.desired = Some((1, key.clone()));
    preview.reset();
    sender
        .send(CompositeRenderResult {
            worker: 0,
            sequence: 1,
            key,
            raster_sources: Arc::clone(&snapshot),
            result: Ok(image::DynamicImage::new_rgba8(1, 1)),
        })
        .unwrap();
    drop(snapshot);
    assert!(weak_snapshot.upgrade().is_some());

    preview.poll(&egui::Context::default());

    assert!(!preview.workers_busy[0]);
    assert!(preview.ready.is_none());
    assert!(weak_snapshot.upgrade().is_none());
}

#[test]
fn active_key_is_stable_for_same_snapshot_and_changes_for_referenced_provider() {
    let path = PathBuf::from("active.jpg");
    let mut document = Document::new("Epoch", 4, 4);
    document.layers.push(Layer {
        kind: LayerKind::Raster {
            path: path.clone(),
            original_path: None,
        },
        ..Layer::default()
    });
    let geometry = geometry(
        Rect::from_min_size(Pos2::ZERO, Vec2::splat(4.0)),
        Rect::from_min_size(Pos2::ZERO, Vec2::splat(4.0)),
    );
    let active = provider_snapshot(path.clone(), 20, [1, 2, 3, 255], (4, 4));
    let unchanged =
        CompositePreviewKey::new_with_sources(1, 0, &document, geometry, 1.0, active.as_ref())
            .unwrap();
    let same =
        CompositePreviewKey::new_with_sources(1, 0, &document, geometry, 1.0, active.as_ref())
            .unwrap();
    assert_eq!(unchanged, same);

    let replacement = provider_snapshot(path, 21, [4, 5, 6, 255], (4, 4));
    let changed =
        CompositePreviewKey::new_with_sources(1, 0, &document, geometry, 1.0, replacement.as_ref())
            .unwrap();
    assert_ne!(unchanged, changed);
}
