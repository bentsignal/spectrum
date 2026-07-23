use super::*;
use crate::CropRect;
use image::{GenericImageView, Rgba, RgbaImage};
use std::{
    fs,
    sync::atomic::{AtomicU64, AtomicUsize, Ordering},
};

static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(1);

struct TestDirectory(std::path::PathBuf);

impl TestDirectory {
    fn new(label: &str) -> Self {
        let unique = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "lumen-preview-{label}-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn identity(id: u64, epoch: u64, photo_id: u64) -> PreviewRequestIdentity {
    PreviewRequestIdentity {
        id,
        epoch,
        photo_id,
        adjustments: Adjustments::default(),
        kind: PreviewRequestKind::Prefetch,
    }
}

fn photo(id: u64) -> Photo {
    Photo::new(id, format!("{id}.jpg").into(), format!("{id}.jpg"), 1, 1)
}

fn prepared(id: u64, adjustments: Adjustments) -> PreparedPreview {
    let rendered = DynamicImage::new_rgba8(2, 2);
    PreparedPreview {
        photo_id: id,
        adjustments,
        source: rendered.clone(),
        fast_source: rendered.clone(),
        histogram: PreviewHistogram::from_image(&rendered),
        rendered,
    }
}

fn track(pipeline: &mut PreviewPipeline, request: PreviewRequestIdentity) {
    pipeline.track_enqueue(PreviewEnqueue {
        accepted: Some(request),
        evicted: vec![],
    });
}

fn immediate_worker() -> PreviewWorker {
    PreviewWorker::with_preparer(Arc::new(|photo, adjustments| {
        Ok(prepared(photo.id, adjustments))
    }))
}

#[test]
fn prepared_preview_applies_the_limit_after_geometry_and_publishes_matching_histogram() {
    let directory = TestDirectory::new("post-geometry");
    let path = directory.0.join("source.png");
    RgbaImage::from_fn(60, 40, |x, y| {
        Rgba([(x * 3) as u8, (y * 5) as u8, (x + y) as u8, 255])
    })
    .save(&path)
    .unwrap();
    let photo = Photo::new(1, path, "source.png".into(), 60, 40);
    let adjustments = Adjustments {
        crop: Some(CropRect {
            x: 0.4,
            y: 0.0,
            width: 0.2,
            height: 1.0,
        }),
        ..Default::default()
    };

    let prepared = prepare_preview_at_size(&photo, adjustments.clone(), 18, 10).unwrap();
    let expected = crate::engine::render_settled_preview(&photo, adjustments, 18).unwrap();

    assert_eq!(prepared.source.dimensions(), (18, 12));
    assert_eq!(prepared.fast_source.dimensions(), (10, 7));
    assert_eq!(prepared.rendered.dimensions(), (5, 18));
    assert_eq!(prepared.rendered.to_rgba8(), expected.to_rgba8());
    assert_eq!(
        prepared.histogram,
        PreviewHistogram::from_image(&prepared.rendered)
    );
}

#[test]
fn raw_prefetch_is_rejected_and_selected_raw_development_has_one_active_lane() {
    let active = Arc::new(AtomicUsize::new(0));
    let maximum = Arc::new(AtomicUsize::new(0));
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let (started_sender, started_receiver) = mpsc::channel();
    let worker_active = Arc::clone(&active);
    let worker_maximum = Arc::clone(&maximum);
    let worker_gate = Arc::clone(&gate);
    let worker = PreviewWorker::with_preparer(Arc::new(move |photo, adjustments| {
        let active_now = worker_active.fetch_add(1, Ordering::SeqCst) + 1;
        worker_maximum.fetch_max(active_now, Ordering::SeqCst);
        started_sender.send(photo.id).unwrap();
        let (gate, released) = &*worker_gate;
        let mut release = gate.lock().unwrap();
        while !*release {
            release = released.wait(release).unwrap();
        }
        worker_active.fetch_sub(1, Ordering::SeqCst);
        Ok(prepared(photo.id, adjustments))
    }));
    let mut first = photo(1);
    first.path = "1.arw".into();
    first.format = "arw".into();
    let mut second = photo(2);
    second.path = "2.arw".into();
    second.format = "arw".into();

    assert!(
        worker
            .request_prefetch(0, first.clone(), Adjustments::default())
            .accepted
            .is_none(),
        "RAW must never enter the speculative authoritative prefetch lane"
    );
    assert!(
        started_receiver
            .recv_timeout(Duration::from_millis(50))
            .is_err()
    );

    assert!(
        worker
            .request_selected(1, 0, first, Adjustments::default())
            .accepted
            .is_some()
    );
    assert_eq!(
        started_receiver
            .recv_timeout(Duration::from_secs(1))
            .unwrap(),
        1
    );
    assert!(
        worker
            .request_selected(2, 0, second, Adjustments::default())
            .accepted
            .is_some()
    );

    let (gate, released) = &*gate;
    *gate.lock().unwrap() = true;
    released.notify_all();
    assert!(worker.recv_timeout(Duration::from_secs(1)).is_ok());
    assert_eq!(
        started_receiver
            .recv_timeout(Duration::from_secs(1))
            .unwrap(),
        2
    );
    assert!(worker.recv_timeout(Duration::from_secs(1)).is_ok());
    assert_eq!(maximum.load(Ordering::SeqCst), 1);
    assert_eq!(active.load(Ordering::SeqCst), 0);
}

#[test]
#[ignore = "manual release-mode probe requiring LUMEN_RAW_PREVIEW_SAMPLE"]
fn selected_raw_worker_memory_and_histogram_probe() {
    let path = std::env::var_os("LUMEN_RAW_PREVIEW_SAMPLE")
        .map(std::path::PathBuf::from)
        .expect("set LUMEN_RAW_PREVIEW_SAMPLE to an immutable 6000x4000 Sony ARW");
    let original_metadata = fs::metadata(&path).unwrap();
    let photo = Photo::new(
        1,
        path.clone(),
        path.file_name().unwrap().to_string_lossy().into_owned(),
        6_000,
        4_000,
    );
    let worker = PreviewWorker::new();
    let mut pipeline = PreviewPipeline::default();
    let adjustments = Adjustments::default();
    let generation = pipeline.select(photo.id, adjustments.clone());
    pipeline.track_enqueue(worker.request_selected(
        generation,
        pipeline.epoch(),
        photo,
        adjustments,
    ));

    let completion = worker.recv_timeout(Duration::from_secs(10)).unwrap();
    let PreviewCompletionDisposition::Publish(prepared) =
        pipeline.complete(completion, Instant::now())
    else {
        panic!("selected RAW should publish atomically");
    };
    assert_eq!(prepared.rendered.dimensions(), (1_800, 1_200));
    assert_eq!(prepared.source.dimensions(), (1_800, 1_200));
    assert_eq!(prepared.fast_source.dimensions(), (960, 640));
    let source_bytes = prepared.source.as_bytes().len();
    let fast_source_bytes = prepared.fast_source.as_bytes().len();
    let rendered_bytes = prepared.rendered.as_bytes().len();
    let retained_pixel_bytes = source_bytes + fast_source_bytes + rendered_bytes;
    assert!(source_bytes <= 1_800 * 1_200 * 4);
    assert!(fast_source_bytes <= 960 * 640 * 4);
    assert!(rendered_bytes <= 1_800 * 1_200 * 4);
    assert!(retained_pixel_bytes <= 1_800 * 1_200 * 8 + 960 * 640 * 4);
    assert!(
        prepared.histogram.luma.iter().sum::<u32>() > 0,
        "published RAW preview must include a populated histogram"
    );
    assert!(
        pipeline.cache.is_empty(),
        "the selected RAW publication must move directly to the GUI, not retain a cache copy"
    );
    eprintln!(
        "selected RAW retained buffers: source={source_bytes} bytes, fast={fast_source_bytes} bytes, rendered={rendered_bytes} bytes, total={retained_pixel_bytes} bytes"
    );

    let final_metadata = fs::metadata(path).unwrap();
    assert_eq!(final_metadata.len(), original_metadata.len());
    assert_eq!(
        final_metadata.modified().unwrap(),
        original_metadata.modified().unwrap()
    );
}

#[test]
fn prefetch_dedup_includes_epoch_and_evicts_the_oldest_request() {
    let mut lane = PreviewLane::new(2);
    lane.active = Some(identity(1, 4, 8));
    let duplicate = PreviewJob {
        identity: identity(2, 4, 8),
        photo: photo(8),
    };
    assert!(lane.enqueue_prefetch(duplicate).accepted.is_none());

    for (request_id, epoch, photo_id) in [(3, 5, 8), (4, 5, 9)] {
        let outcome = lane.enqueue_prefetch(PreviewJob {
            identity: identity(request_id, epoch, photo_id),
            photo: photo(photo_id),
        });
        assert!(outcome.accepted.is_some());
    }
    let outcome = lane.enqueue_prefetch(PreviewJob {
        identity: identity(5, 5, 10),
        photo: photo(10),
    });
    assert_eq!(
        outcome
            .evicted
            .iter()
            .map(|request| request.id)
            .collect::<Vec<_>>(),
        vec![3]
    );
    assert_eq!(
        lane.pending
            .iter()
            .map(|job| job.identity.id)
            .collect::<Vec<_>>(),
        vec![4, 5]
    );
}

#[test]
fn catalog_purge_removes_only_stale_queued_work() {
    let mut lane = PreviewLane::new(4);
    for (request_id, epoch) in [(1, 2), (2, 3), (3, 2)] {
        lane.enqueue_latest(PreviewJob {
            identity: identity(request_id, epoch, request_id),
            photo: photo(request_id),
        });
    }
    let purged = lane.purge_other_epochs(3);
    assert_eq!(
        purged.iter().map(|request| request.id).collect::<Vec<_>>(),
        vec![1, 3]
    );
    assert_eq!(lane.pending.front().unwrap().identity.id, 2);
}

#[test]
fn stale_selected_generation_cannot_publish_over_current_photo() {
    let mut selection = PreviewSelection::default();
    let adjustments = Adjustments::default();
    let stale_generation = selection.select(7, adjustments.clone());
    let current_generation = selection.select(7, adjustments.clone());
    let mut stale = identity(1, selection.epoch(), 7);
    stale.kind = PreviewRequestKind::Selected {
        generation: stale_generation,
    };
    let mut current = stale.clone();
    current.id = 2;
    current.kind = PreviewRequestKind::Selected {
        generation: current_generation,
    };
    assert!(!selection.can_publish(&stale));
    assert!(selection.can_publish(&current));
}

#[test]
fn pipeline_tracks_acceptance_eviction_completion_and_publication() {
    let mut pipeline = PreviewPipeline::default();
    let adjustments = Adjustments::default();
    let generation = pipeline.select(7, adjustments.clone());
    let first = identity(1, pipeline.epoch(), 6);
    let mut selected = identity(2, pipeline.epoch(), 7);
    selected.kind = PreviewRequestKind::Selected { generation };
    pipeline.track_enqueue(PreviewEnqueue {
        accepted: Some(first.clone()),
        evicted: vec![],
    });
    pipeline.track_enqueue(PreviewEnqueue {
        accepted: Some(selected.clone()),
        evicted: vec![first],
    });
    assert_eq!(pipeline.outstanding.len(), 1);
    assert!(matches!(
        pipeline.complete(
            PreviewCompletion {
                identity: selected,
                result: Ok(prepared(7, adjustments)),
            },
            Instant::now(),
        ),
        PreviewCompletionDisposition::Publish(_)
    ));
    assert!(pipeline.outstanding.is_empty());
}

#[test]
fn queued_adjacent_target_is_promoted_around_unrelated_active_prefetch() {
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let (started_sender, started_receiver) = mpsc::channel();
    let preparer_gate = Arc::clone(&gate);
    let worker = PreviewWorker::with_preparer(Arc::new(move |photo, adjustments| {
        if photo.id == 1 {
            started_sender.send(()).unwrap();
            let (gate, released) = &*preparer_gate;
            let mut release = gate.lock().unwrap();
            while !*release {
                release = released.wait(release).unwrap();
            }
        }
        Ok(prepared(photo.id, adjustments))
    }));
    let mut pipeline = PreviewPipeline::default();
    pipeline.track_enqueue(worker.request_prefetch(
        pipeline.epoch(),
        photo(1),
        Adjustments::default(),
    ));
    started_receiver
        .recv_timeout(Duration::from_secs(1))
        .unwrap();
    let target = worker.request_prefetch(pipeline.epoch(), photo(2), Adjustments::default());
    let target_id = target.accepted.as_ref().unwrap().id;
    pipeline.track_enqueue(target);
    pipeline.select(2, Adjustments::default());

    assert_eq!(
        pipeline.request_decision(&worker, Instant::now(), 2, &Adjustments::default()),
        PreviewRequestDecision::Promoted
    );
    let completion = worker.recv_timeout(Duration::from_secs(1)).unwrap();
    assert_eq!(completion.identity.id, target_id);
    assert!(matches!(
        completion.identity.kind,
        PreviewRequestKind::Selected { .. }
    ));
    assert!(matches!(
        pipeline.complete(completion, Instant::now()),
        PreviewCompletionDisposition::Publish(_)
    ));

    let (gate, released) = &*gate;
    *gate.lock().unwrap() = true;
    released.notify_one();
}

#[test]
fn promotion_claim_race_produces_exactly_one_publishable_completion() {
    for _ in 0..16 {
        let decode_count = Arc::new(AtomicUsize::new(0));
        let worker_count = Arc::clone(&decode_count);
        let worker = PreviewWorker::with_preparer(Arc::new(move |photo, adjustments| {
            worker_count.fetch_add(1, Ordering::SeqCst);
            Ok(prepared(photo.id, adjustments))
        }));
        let mut pipeline = PreviewPipeline::default();
        let adjustments = Adjustments::default();
        let enqueue = worker.request_prefetch(pipeline.epoch(), photo(7), adjustments.clone());
        let request_id = enqueue.accepted.as_ref().unwrap().id;
        pipeline.track_enqueue(enqueue);
        pipeline.select(7, adjustments.clone());
        assert!(matches!(
            pipeline.request_decision(&worker, Instant::now(), 7, &adjustments),
            PreviewRequestDecision::Promoted
                | PreviewRequestDecision::ReusedActivePrefetch
                | PreviewRequestDecision::Pending
        ));

        let completion = worker.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(completion.identity.id, request_id);
        assert!(matches!(
            pipeline.complete(completion, Instant::now()),
            PreviewCompletionDisposition::Publish(_)
        ));
        assert_eq!(decode_count.load(Ordering::SeqCst), 1);
        assert!(matches!(
            worker.recv_timeout(Duration::from_millis(10)),
            Err(RecvTimeoutError::Timeout)
        ));
    }
}

#[test]
fn active_matching_prefetch_is_reused_without_duplicate_decode() {
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let decode_count = Arc::new(AtomicUsize::new(0));
    let (started_sender, started_receiver) = mpsc::channel();
    let preparer_gate = Arc::clone(&gate);
    let worker_count = Arc::clone(&decode_count);
    let worker = PreviewWorker::with_preparer(Arc::new(move |photo, adjustments| {
        worker_count.fetch_add(1, Ordering::SeqCst);
        started_sender.send(()).unwrap();
        let (gate, released) = &*preparer_gate;
        let mut release = gate.lock().unwrap();
        while !*release {
            release = released.wait(release).unwrap();
        }
        Ok(prepared(photo.id, adjustments))
    }));
    let mut pipeline = PreviewPipeline::default();
    let adjustments = Adjustments::default();
    let enqueue = worker.request_prefetch(pipeline.epoch(), photo(7), adjustments.clone());
    let request_id = enqueue.accepted.as_ref().unwrap().id;
    pipeline.track_enqueue(enqueue);
    started_receiver
        .recv_timeout(Duration::from_secs(1))
        .unwrap();
    pipeline.select(7, adjustments.clone());

    assert_eq!(
        pipeline.request_decision(&worker, Instant::now(), 7, &adjustments),
        PreviewRequestDecision::ReusedActivePrefetch
    );
    assert_eq!(decode_count.load(Ordering::SeqCst), 1);
    let (gate, released) = &*gate;
    *gate.lock().unwrap() = true;
    released.notify_one();
    let completion = worker.recv_timeout(Duration::from_secs(1)).unwrap();
    assert_eq!(completion.identity.id, request_id);
    assert!(matches!(
        pipeline.complete(completion, Instant::now()),
        PreviewCompletionDisposition::Publish(_)
    ));
    assert_eq!(decode_count.load(Ordering::SeqCst), 1);
    assert!(matches!(
        worker.recv_timeout(Duration::from_millis(50)),
        Err(RecvTimeoutError::Timeout)
    ));
}

#[test]
fn canceled_and_untracked_completions_are_ignored() {
    let mut pipeline = PreviewPipeline::default();
    let adjustments = Adjustments::default();
    let generation = pipeline.select(7, adjustments.clone());
    let mut untracked = identity(1, pipeline.epoch(), 7);
    untracked.kind = PreviewRequestKind::Selected { generation };
    assert!(matches!(
        pipeline.complete(
            PreviewCompletion {
                identity: untracked.clone(),
                result: Ok(prepared(7, adjustments.clone())),
            },
            Instant::now(),
        ),
        PreviewCompletionDisposition::Ignored
    ));

    track(&mut pipeline, untracked.clone());
    pipeline.track_enqueue(PreviewEnqueue {
        accepted: None,
        evicted: vec![untracked.clone()],
    });
    assert!(matches!(
        pipeline.complete(
            PreviewCompletion {
                identity: untracked,
                result: Ok(prepared(7, adjustments)),
            },
            Instant::now(),
        ),
        PreviewCompletionDisposition::Ignored
    ));
    assert!(pipeline.cache.is_empty());
}

#[test]
fn catalog_reset_purges_queued_worker_jobs_and_ignores_active_old_epoch() {
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let (started_sender, started_receiver) = mpsc::channel();
    let preparer_gate = Arc::clone(&gate);
    let preparer = Arc::new(move |photo: &Photo, adjustments: Adjustments| {
        if photo.id == 1 {
            started_sender.send(()).unwrap();
            let (gate, released) = &*preparer_gate;
            let mut release = gate.lock().unwrap();
            while !*release {
                release = released.wait(release).unwrap();
            }
        }
        Ok(prepared(photo.id, adjustments))
    });
    let worker = PreviewWorker::with_preparer(preparer);
    let mut pipeline = PreviewPipeline::default();
    pipeline.track_enqueue(worker.request_prefetch(
        pipeline.epoch(),
        photo(1),
        Adjustments::default(),
    ));
    started_receiver
        .recv_timeout(Duration::from_secs(1))
        .unwrap();
    pipeline.track_enqueue(worker.request_prefetch(
        pipeline.epoch(),
        photo(2),
        Adjustments::default(),
    ));
    pipeline.reset_catalog(&worker);
    assert!(pipeline.outstanding.is_empty());

    let (gate, released) = &*gate;
    *gate.lock().unwrap() = true;
    released.notify_one();
    let completion = worker.recv_timeout(Duration::from_secs(1)).unwrap();
    assert!(matches!(
        pipeline.complete(completion, Instant::now()),
        PreviewCompletionDisposition::Ignored
    ));
    assert!(matches!(
        worker.recv_timeout(Duration::from_millis(50)),
        Err(RecvTimeoutError::Timeout)
    ));
}

#[test]
fn pipeline_bounds_prepared_and_failure_state() {
    let now = Instant::now();
    let mut pipeline = PreviewPipeline::default();
    for id in 1..=10 {
        pipeline.select(99, Adjustments::default());
        let request = identity(id, pipeline.epoch(), id);
        track(&mut pipeline, request.clone());
        assert!(matches!(
            pipeline.complete(
                PreviewCompletion {
                    identity: request,
                    result: Ok(prepared(id, Adjustments::default())),
                },
                now,
            ),
            PreviewCompletionDisposition::Cached
        ));
    }
    assert_eq!(pipeline.cache.len(), PREVIEW_CACHE_CAPACITY);

    for id in 1..=10 {
        let generation = pipeline.select(id, Adjustments::default());
        let mut request = identity(100 + id, pipeline.epoch(), id);
        request.kind = PreviewRequestKind::Selected { generation };
        track(&mut pipeline, request.clone());
        assert!(matches!(
            pipeline.complete(
                PreviewCompletion {
                    identity: request,
                    result: Err("missing source".into()),
                },
                now,
            ),
            PreviewCompletionDisposition::Failed(_)
        ));
    }
    assert_eq!(pipeline.failures.len(), FAILURE_CACHE_CAPACITY);
}

#[test]
fn permanent_errors_back_off_exponentially_without_eight_ms_resubmission() {
    let now = Instant::now();
    let worker = immediate_worker();
    let mut pipeline = PreviewPipeline::default();
    let adjustments = Adjustments::default();
    let generation = pipeline.select(7, adjustments.clone());
    let mut failed = identity(1, pipeline.epoch(), 7);
    failed.kind = PreviewRequestKind::Selected { generation };
    track(&mut pipeline, failed.clone());
    assert!(matches!(
        pipeline.complete(
            PreviewCompletion {
                identity: failed,
                result: Err("corrupt JPEG".into()),
            },
            now,
        ),
        PreviewCompletionDisposition::Failed(_)
    ));
    assert_eq!(
        pipeline.request_decision(&worker, now + Duration::from_millis(8), 7, &adjustments),
        PreviewRequestDecision::Backoff(Duration::from_millis(242))
    );
    assert_eq!(
        pipeline.request_decision(&worker, now + FAILURE_RETRY_BASE, 7, &adjustments),
        PreviewRequestDecision::Request
    );
}

#[test]
fn selected_lane_bypasses_blocked_prefetch_and_drop_does_not_join() {
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let (started_sender, started_receiver) = mpsc::channel();
    let preparer_gate = Arc::clone(&gate);
    let preparer = Arc::new(move |photo: &Photo, adjustments: Adjustments| {
        if photo.id == 1 {
            started_sender.send(()).unwrap();
            let (gate, released) = &*preparer_gate;
            let mut release = gate.lock().unwrap();
            while !*release {
                release = released.wait(release).unwrap();
            }
        }
        Ok(prepared(photo.id, adjustments))
    });
    let worker = PreviewWorker::with_preparer(preparer);
    assert!(
        worker
            .request_prefetch(0, photo(1), Adjustments::default())
            .accepted
            .is_some()
    );
    started_receiver
        .recv_timeout(Duration::from_secs(1))
        .unwrap();
    let selected = worker
        .request_selected(1, 0, photo(2), Adjustments::default())
        .accepted
        .unwrap();
    let completion = worker.recv_timeout(Duration::from_secs(1)).unwrap();
    assert_eq!(completion.identity.id, selected.id);

    let drop_started = Instant::now();
    drop(worker);
    assert!(drop_started.elapsed() < Duration::from_millis(50));
    let (gate, released) = &*gate;
    *gate.lock().unwrap() = true;
    released.notify_one();
}
