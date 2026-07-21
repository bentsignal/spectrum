use std::sync::atomic::{AtomicUsize, Ordering};

use super::{tests::*, *};
use prism_core::Layer;

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
                path: request.path,
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
        |_path, _identity| PreparationOutcome::LegacyNative,
    );
    let mut coordinator = RasterSourceCoordinator {
        request_sender: Some(request_sender.clone()),
        result_receiver,
        tab_paths: HashMap::from([(1, HashSet::from([active_path.clone()]))]),
        paths: HashMap::from([(
            active_path.clone(),
            PathState {
                generation: 2,
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
    assert!(coordinator.snapshot.legacy_native.contains(&active_path));
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
