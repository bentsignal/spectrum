use std::sync::atomic::{AtomicUsize, Ordering};

use super::{tests::*, *};
use prism_core::{
    BrushProgram, BrushSample, BrushStroke, BrushStyle, Layer, SampledSourceSnapshot, Transform,
};
use spectrum_imaging::Adjustments;

fn exact_requirement_document(path: PathBuf, content_sha256: String) -> Document {
    let snapshot = SampledSourceSnapshot {
        version: prism_core::SAMPLED_SOURCE_VERSION,
        source_layer_id: 1,
        source_layer_name: "Hidden source".into(),
        path: path.clone(),
        content_hash: content_sha256,
        width: 4,
        height: 4,
        anchor_local: [1.5, 1.5],
        source_transform: Transform::default(),
        adjustments: Adjustments::default(),
        pixel_mask: None,
        vector_mask: None,
    };
    let marker = BrushStroke::new_clone_stamp(
        BrushStyle::default(),
        [BrushSample {
            x: 1.5,
            y: 1.5,
            pressure: 1.0,
        }],
        snapshot.clone(),
    )
    .unwrap();
    let source_id = serde_json::to_value(&marker).unwrap()["source"]["source_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let mut document = raster_document(path);
    document.layers[0].id = 1;
    document.layers[0].visible = false;
    document.layers.push(Layer {
        id: 2,
        name: "Clone".into(),
        kind: LayerKind::Paint {
            program: BrushProgram::new(4, 4).unwrap().append(marker).unwrap(),
        },
        ..Layer::default()
    });
    document.next_id = 3;
    let mut encoded = serde_json::to_value(document).unwrap();
    encoded["clone_source"] = serde_json::json!(source_id);
    encoded["sampled_sources"] = serde_json::json!({(source_id): snapshot});
    serde_json::from_value(encoded).unwrap()
}

#[test]
fn cache_root_includes_format_compatibility_and_app_version() {
    assert_eq!(
        derived_cache_root(Path::new("/cache/Prism"), "4.2.1"),
        PathBuf::from("/cache/Prism/Derived Raster Backings/derived-rgba8-schema-v2/4.2.1")
    );
}

#[test]
fn generic_failures_stop_with_the_last_diagnostic() {
    let document = raster_document("broken.jpg");
    let path = PathBuf::from("broken.jpg");
    let (mut coordinator, requests) = detached_coordinator_with_capacity(1);
    coordinator.set_tab_document(1, &document);
    coordinator.set_active_tab(1);
    let mut request = requests.try_recv().unwrap();
    let mut now = Instant::now();

    for attempt in 1..=MAX_GENERIC_FAILURE_ATTEMPTS {
        coordinator.apply_result(
            PreparationResult {
                path: request.path.clone(),
                generation: request.generation,
                attempts: request.attempts,
                outcome: PreparationOutcome::Failed(format!("failure {attempt}")),
            },
            now,
        );
        if attempt < MAX_GENERIC_FAILURE_ATTEMPTS {
            now += preparation_retry_delay(attempt);
            coordinator.dispatch_ready(now);
            request = requests.try_recv().unwrap();
        }
    }

    assert!(matches!(
        &coordinator.paths[&path].phase,
        PathPhase::Failed { diagnostic } if diagnostic == "failure 3"
    ));
    assert_eq!(
        coordinator.terminal_failure(),
        Some((path.clone(), "failure 3".into()))
    );
    assert_eq!(
        terminal_failure_status(&path, "failure 3"),
        "Bounded preview failed for broken.jpg: failure 3"
    );
    coordinator.dispatch_ready(now + MAX_RETRY_DELAY + Duration::from_secs(1));
    assert!(requests.try_recv().is_err());
    coordinator.set_tab_document(1, &document);
    assert!(requests.try_recv().is_err());

    let terminal_generation = coordinator.paths[&path].generation;
    assert_eq!(coordinator.retry_terminal_failures(), 1);
    let retry = requests.try_recv().unwrap();
    assert_ne!(retry.generation, terminal_generation);
    assert_eq!(coordinator.retry_terminal_failures(), 0);
    assert!(requests.try_recv().is_err());
}

#[test]
fn retrying_failure_does_not_starve_other_active_source() {
    let mut document = raster_document("first.jpg");
    let mut second = raster_document("second.jpg");
    document.layers.push(second.layers.remove(0));
    let (mut coordinator, requests) = detached_coordinator_with_capacity(1);
    coordinator.set_tab_document(1, &document);
    coordinator.set_active_tab(1);
    let failed = requests.try_recv().unwrap();
    coordinator.apply_result(
        PreparationResult {
            path: failed.path.clone(),
            generation: failed.generation,
            attempts: 0,
            outcome: PreparationOutcome::Failed("temporary".into()),
        },
        Instant::now(),
    );
    coordinator.dispatch_ready(Instant::now());
    let next = requests.try_recv().unwrap();
    assert_ne!(next.path, failed.path);
}

#[test]
fn hidden_and_inactive_sources_do_not_prepare_or_change_active_epoch() {
    let active_path = PathBuf::from("active.jpg");
    let active = raster_document(active_path.clone());
    let mut hidden = raster_document("hidden.jpg");
    hidden.layers[0].visible = false;
    let inactive = raster_document("inactive.jpg");
    let (mut coordinator, requests) = detached_coordinator();

    coordinator.set_tab_document(1, &active);
    coordinator.set_active_tab(1);
    let request = requests.try_recv().unwrap();
    coordinator.apply_result(
        PreparationResult {
            path: request.path,
            generation: request.generation,
            attempts: 0,
            outcome: PreparationOutcome::Ready(resolved("active", None)),
        },
        Instant::now(),
    );
    let epoch = coordinator.snapshot.epoch;
    let snapshot = coordinator.snapshot();

    coordinator.set_tab_document(2, &inactive);
    coordinator.set_tab_document(3, &hidden);

    assert_eq!(coordinator.snapshot.epoch, epoch);
    assert!(Arc::ptr_eq(&snapshot, &coordinator.snapshot));
    assert!(requests.try_recv().is_err());
    assert_eq!(coordinator.paths.len(), 1);
    assert!(coordinator.paths.contains_key(&active_path));
}

#[test]
fn active_switch_releases_old_provider_and_prepares_new_visible_source() {
    let first_path = PathBuf::from("first.jpg");
    let second_path = PathBuf::from("second.jpg");
    let first = raster_document(first_path.clone());
    let second = raster_document(second_path.clone());
    let drops = Arc::new(AtomicUsize::new(0));
    let (mut coordinator, requests) = detached_coordinator();
    coordinator.set_tab_document(1, &first);
    coordinator.set_tab_document(2, &second);
    assert!(requests.try_recv().is_err());

    coordinator.set_active_tab(1);
    let first_request = requests.try_recv().unwrap();
    coordinator.apply_result(
        PreparationResult {
            path: first_request.path,
            generation: first_request.generation,
            attempts: 0,
            outcome: PreparationOutcome::Ready(resolved("first", Some(Arc::clone(&drops)))),
        },
        Instant::now(),
    );
    assert!(coordinator.snapshot.resolve(&first_path).is_some());

    coordinator.set_active_tab(2);

    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert!(coordinator.snapshot.resolve(&first_path).is_none());
    let active_generations = coordinator
        .active_generations
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert!(!active_generations.contains_key(&first_path));
    assert!(active_generations.contains_key(&second_path));
    drop(active_generations);
    assert_eq!(requests.try_recv().unwrap().path, second_path);
}

#[test]
fn atomic_active_replacement_preserves_overlapping_ready_provider() {
    let path = PathBuf::from("shared.jpg");
    let document = raster_document(path.clone());
    let (mut coordinator, requests) = detached_coordinator();
    coordinator.set_tab_document(1, &document);
    coordinator.set_tab_document(2, &document);
    coordinator.set_active_tab(1);
    let request = requests.try_recv().unwrap();
    coordinator.apply_result(
        PreparationResult {
            path: request.path,
            generation: request.generation,
            attempts: 0,
            outcome: PreparationOutcome::Ready(resolved("shared", None)),
        },
        Instant::now(),
    );
    let snapshot = coordinator.snapshot();
    let epoch = snapshot.epoch;
    let generation = coordinator.paths[&path].generation;

    coordinator.set_active_tab(2);
    coordinator.remove_tab(1);

    assert_eq!(coordinator.snapshot.epoch, epoch);
    assert!(Arc::ptr_eq(&snapshot, &coordinator.snapshot));
    assert_eq!(coordinator.paths[&path].generation, generation);
    assert!(coordinator.snapshot.resolve(&path).is_some());
    assert!(requests.try_recv().is_err());
}

#[test]
fn same_path_exact_identity_change_evicts_rejects_and_reprepares() {
    let path = PathBuf::from("same-path.png");
    let digest_a = "aa".repeat(32);
    let digest_b = "bb".repeat(32);
    let document_a = exact_requirement_document(path.clone(), digest_a.clone());
    let document_b = exact_requirement_document(path.clone(), digest_b.clone());
    let (mut coordinator, requests) = detached_coordinator();

    coordinator.set_tab_document(1, &document_a);
    coordinator.set_active_tab(1);
    let request_a = requests.try_recv().unwrap();
    let generation_a = request_a.generation;
    coordinator.apply_result(
        PreparationResult {
            path: request_a.path,
            generation: request_a.generation,
            attempts: 0,
            outcome: PreparationOutcome::Ready(resolved_authenticated("source-a", &digest_a)),
        },
        Instant::now(),
    );
    assert_eq!(
        coordinator
            .snapshot
            .resolve(&path)
            .unwrap()
            .content_sha256(),
        Some(digest_a.as_str())
    );

    coordinator.set_tab_document(1, &document_b);
    let request_b = requests.try_recv().unwrap();
    assert_ne!(generation_a, request_b.generation);
    assert!(coordinator.snapshot.resolve(&path).is_none());

    coordinator.apply_result(
        PreparationResult {
            path: request_b.path,
            generation: request_b.generation,
            attempts: 0,
            outcome: PreparationOutcome::Ready(resolved_authenticated("stale-a", &digest_a)),
        },
        Instant::now(),
    );
    assert!(coordinator.snapshot.resolve(&path).is_none());
    assert!(matches!(
        &coordinator.paths[&path].phase,
        PathPhase::Failed { diagnostic }
            if diagnostic.contains("exact required content identity")
    ));

    assert_eq!(coordinator.retry_terminal_failures(), 1);
    let retry_b = requests.try_recv().unwrap();
    coordinator.apply_result(
        PreparationResult {
            path: retry_b.path,
            generation: retry_b.generation,
            attempts: retry_b.attempts,
            outcome: PreparationOutcome::Ready(resolved_authenticated("source-b", &digest_b)),
        },
        Instant::now(),
    );
    let exact_b = coordinator.snapshot.resolve(&path).unwrap();
    assert_eq!(exact_b.content_sha256(), Some(digest_b.as_str()));
    assert_eq!(exact_b.source_epoch().as_str(), "source-b");

    let full =
        prism_core::render_document_with_sources(&document_b, None, coordinator.snapshot.as_ref())
            .unwrap()
            .to_rgba8();
    let region = prism_core::render_document_region_scaled_with_sources(
        &document_b,
        1.0,
        prism_core::RenderRegion {
            x: 0,
            y: 0,
            width: 4,
            height: 4,
        },
        coordinator.snapshot.as_ref(),
    )
    .unwrap()
    .to_rgba8();
    assert_eq!(region, full);
    assert!(full.pixels().any(|pixel| pixel[3] > 0));
    let reopened: Document =
        serde_json::from_slice(&serde_json::to_vec(&document_b).unwrap()).unwrap();
    assert_eq!(
        prism_core::render_document_with_sources(&reopened, None, coordinator.snapshot.as_ref(),)
            .unwrap()
            .to_rgba8(),
        full
    );
    let export = std::env::temp_dir().join(format!(
        "prism-same-path-clone-{}-{}.png",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    prism_core::export_document_with_sources(
        &document_b,
        &export,
        92,
        coordinator.snapshot.as_ref(),
    )
    .unwrap();
    assert_eq!(image::open(&export).unwrap().to_rgba8(), full);
    std::fs::remove_file(export).unwrap();
}

#[test]
fn conflicting_exact_identities_for_one_path_fail_closed() {
    let digest_a = "aa".repeat(32);
    let digest_b = "bb".repeat(32);
    let mut requirement = ContentRequirement::Any;
    requirement.merge(Some(digest_a));
    requirement.merge(Some(digest_b));
    assert_eq!(requirement, ContentRequirement::ConflictingExactIdentities);
    assert!(!requirement.accepts(&resolved_authenticated("either-source", &"aa".repeat(32))));
    assert!(requirement.conflict_diagnostic().is_some());
}

#[test]
fn stale_saturated_worker_queue_wakes_poll_to_dispatch_new_active_work() {
    let stale_path = PathBuf::from("stale.jpg");
    let active_path = PathBuf::from("active.jpg");
    let (request_sender, request_receiver) = mpsc::sync_channel(1);
    request_sender
        .try_send(PreparationRequest {
            path: stale_path,
            generation: 1,
            identity: None,
            attempts: 0,
        })
        .unwrap();
    let (result_sender, result_receiver) = mpsc::channel();
    let (wake_sender, wake_receiver) = mpsc::channel();
    let active_generations = Arc::new(Mutex::new(HashMap::from([(active_path.clone(), 2)])));
    let worker = spawn_preparation_worker(
        request_receiver,
        result_sender,
        Arc::clone(&active_generations),
        move || {
            let _ = wake_sender.send(());
        },
        |_path, _identity| PreparationOutcome::Ready(resolved("worker", None)),
    );
    let mut coordinator = RasterSourceCoordinator {
        request_sender: Some(request_sender.clone()),
        result_receiver,
        tab_requirements: HashMap::from([(
            1,
            HashMap::from([(active_path.clone(), ContentRequirement::Any)]),
        )]),
        paths: HashMap::from([(
            active_path.clone(),
            PathState {
                generation: 2,
                requirement: ContentRequirement::Any,
                phase: PathPhase::Needed,
            },
        )]),
        active_tab: Some(1),
        active_generations,
        snapshot: Arc::new(RasterSourceSnapshot::empty()),
        next_generation: 2,
    };
    let context = egui::Context::default();
    wake_receiver
        .recv_timeout(Duration::from_secs(1))
        .expect("stale request skip did not wake the coordinator");
    coordinator.poll(&context);
    wake_receiver
        .recv_timeout(Duration::from_secs(1))
        .expect("new active request completion did not wake the coordinator");
    coordinator.poll(&context);
    assert!(coordinator.snapshot.providers.contains_key(&active_path));
    drop(coordinator);
    drop(request_sender);
    worker.join().unwrap();
}

#[test]
fn zero_opacity_raster_is_not_part_of_the_active_working_set() {
    let mut document = raster_document("transparent.jpg");
    document.layers[0].opacity = 0.0;
    document.layers.push(Layer::default());
    let (mut coordinator, requests) = detached_coordinator();
    coordinator.set_tab_document(1, &document);
    coordinator.set_active_tab(1);
    assert!(coordinator.paths.is_empty());
    assert!(requests.try_recv().is_err());
}
