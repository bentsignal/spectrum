use super::*;
use spectrum_revisions::{Actor, ActorKind};

pub(super) fn benchmark(
    strict: bool,
    profile: BenchmarkProfile,
    raw_import: Option<&std::path::Path>,
) -> Result<serde_json::Value> {
    const PREVIEW_WIDTH: u32 = 1800;
    const PREVIEW_HEIGHT: u32 = 1200;
    const EXPORT_WIDTH: u32 = 6000;
    const EXPORT_HEIGHT: u32 = 4000;
    let directory = std::env::temp_dir().join(format!("lumen-benchmark-{}", std::process::id()));
    if directory.exists() {
        fs::remove_dir_all(&directory)?;
    }
    fs::create_dir_all(&directory)?;
    let result = (|| -> Result<serde_json::Value> {
        let source_path = directory.join("24mp-source.jpg");
        let source = deterministic_rgb(EXPORT_WIDTH, EXPORT_HEIGHT);
        DynamicImage::ImageRgb8(source)
            .save(&source_path)
            .context("prepare deterministic 24 MP benchmark source")?;

        let import_source = directory.join("import-source.jpg");
        DynamicImage::ImageRgb8(deterministic_rgb(2400, 1600))
            .save(&import_source)
            .context("prepare deterministic import benchmark source")?;
        let mut import_paths = Vec::with_capacity(12);
        for index in 0..12 {
            let path = directory.join(format!("import-{index:02}.jpg"));
            fs::copy(&import_source, &path)?;
            import_paths.push(path);
        }
        let mut import_samples = Vec::with_capacity(6);
        for _ in 0..6 {
            let started = Instant::now();
            let mut import_project = Project::new("Import benchmark");
            std::hint::black_box(import_project.import(&import_paths)?);
            import_samples.push(started.elapsed());
        }
        let navigation_photos = prepare_navigation_photos(&directory)?;

        let mut project = Project::new("Performance benchmark");
        let mut photo = Photo::new(
            1,
            source_path.clone(),
            "24mp-source.jpg".into(),
            EXPORT_WIDTH,
            EXPORT_HEIGHT,
        );
        photo.format = "jpg".into();
        project.photos.push(photo);
        project.selected = Some(1);
        let catalog_path = directory.join("benchmark.lumen");
        let session = SessionId::new();
        let actor = Actor {
            id: "benchmark:lumen".into(),
            display_name: "Lumen benchmark".into(),
            kind: ActorKind::System,
        };
        let mut workspace =
            Workspace::create_durable(project, &catalog_path, actor.clone(), session)?;

        let mut command_samples = Vec::with_capacity(12);
        for iteration in 0..12 {
            let started = Instant::now();
            let mut invocation = Workspace::open_as(&catalog_path, actor.clone(), session)?;
            let mut adjustments = invocation.project.photo(1)?.adjustments.clone();
            let y = if iteration % 2 == 0 { 0.42 } else { 0.58 };
            adjustments.curves.master = ToneCurve {
                points: vec![
                    CurvePoint { x: 0.0, y: 0.0 },
                    CurvePoint { x: 0.5, y },
                    CurvePoint { x: 1.0, y: 1.0 },
                ],
            };
            invocation.execute(Command::SetAdjustments { id: 1, adjustments })?;
            command_samples.push(started.elapsed());
            workspace = invocation;
        }

        let preview = DynamicImage::ImageRgb8(deterministic_rgb(PREVIEW_WIDTH, PREVIEW_HEIGHT));
        let preview_adjustments = Adjustments {
            exposure: 0.35,
            contrast: 12.0,
            shadows: 18.0,
            vibrance: 8.0,
            curves: ToneCurves {
                master: ToneCurve {
                    points: vec![
                        CurvePoint { x: 0.0, y: 0.0 },
                        CurvePoint { x: 0.4, y: 0.35 },
                        CurvePoint { x: 1.0, y: 1.0 },
                    ],
                },
                ..Default::default()
            },
            ..Default::default()
        };
        std::hint::black_box(render_image(
            preview.clone(),
            preview_adjustments.clone(),
            RenderOptions::default(),
        ));
        let mut preview_samples = Vec::with_capacity(8);
        for _ in 0..8 {
            let started = Instant::now();
            std::hint::black_box(render_image(
                preview.clone(),
                preview_adjustments.clone(),
                RenderOptions::default(),
            ));
            preview_samples.push(started.elapsed());
        }

        let export_path = directory.join("24mp-export.jpg");
        let started = Instant::now();
        workspace.execute(Command::Export {
            id: 1,
            path: export_path.clone(),
            max_size: None,
            quality: 90,
        })?;
        let export_duration = started.elapsed();
        let export_size = fs::metadata(&export_path)?.len();

        let command = latency_metric(
            "tone_curve_command",
            "Portable project load, core command, and atomic 24 MP project publication",
            &command_samples,
            50.0,
            profile.command_budget_ms(),
        );
        let preview_metric = latency_metric(
            "tone_curve_preview",
            "1800x1200 developed preview frame",
            &preview_samples,
            16.7,
            profile.preview_budget_ms(),
        );
        let export_ms = duration_ms(export_duration);
        let export_pass = export_ms <= 5000.0;
        let export = json!({
            "name": "jpeg_export_24mp",
            "workload": "6000x4000 JPEG decode, develop, and quality-90 encode",
            "elapsed_ms": rounded(export_ms),
            "megapixels_per_second": rounded(24.0 / export_duration.as_secs_f64()),
            "output_bytes": export_size,
            "target_ms": 2000.0,
            "budget_ms": 5000.0,
            "pass": export_pass,
        });
        let jpeg_import = latency_metric(
            "jpeg_batch_import",
            "12 deterministic 2400x1600 JPEG files: validate, dimensions, and catalog records",
            &import_samples,
            75.0,
            250.0,
        );
        let navigation_metrics =
            navigation_switch_metrics(&navigation_photos, &source_path, profile)?;
        let raw_import_metric = raw_import.map(|path| -> Result<serde_json::Value> {
            let mut samples = Vec::with_capacity(3);
            for _ in 0..3 {
                let started = Instant::now();
                let mut project = Project::new("RAW import benchmark");
                std::hint::black_box(project.import(&[path.to_owned()])?);
                samples.push(started.elapsed());
            }
            Ok(latency_metric(
                "raw_metadata_import",
                "Supplied RAW: container validation, dimensions, EXIF, and catalog record (no development)",
                &samples,
                250.0,
                1500.0,
            ))
        }).transpose()?;
        let passed = command["pass"].as_bool() == Some(true)
            && preview_metric["pass"].as_bool() == Some(true)
            && jpeg_import["pass"].as_bool() == Some(true)
            && navigation_metrics
                .iter()
                .all(|metric| metric["pass"].as_bool() == Some(true))
            && raw_import_metric
                .as_ref()
                .is_none_or(|metric| metric["pass"].as_bool() == Some(true))
            && export_pass;
        let mut metrics = vec![command, preview_metric, jpeg_import, export];
        metrics.extend(navigation_metrics);
        if let Some(metric) = raw_import_metric {
            metrics.push(metric);
        }
        let report = json!({
            "ok": true,
            "action": "benchmark",
            "strict": strict,
            "profile": profile.name(),
            "passed": passed,
            "budgets": "Targets describe excellent feel; budgets are CI regression limits.",
            "metrics": metrics,
        });
        if strict && !passed {
            anyhow::bail!(
                "performance budget missed: {}",
                serde_json::to_string(&report)?
            );
        }
        Ok(report)
    })();
    let _ = fs::remove_dir_all(&directory);
    result
}

fn navigation_switch_metrics(
    photos: &[Photo],
    large_prefetch_path: &std::path::Path,
    profile: BenchmarkProfile,
) -> Result<Vec<serde_json::Value>> {
    anyhow::ensure!(
        photos.len() >= 25,
        "navigation benchmark requires twenty-five distinct photos"
    );
    let sequential_dispatch_photos = &photos[..6];
    let nonsequential_dispatch_photos = &photos[6..12];
    let warm_photos = &photos[12..18];
    let cold_photos = &photos[18..24];
    let sequential: Vec<_> = (0..warm_photos.len()).collect();
    let nonsequential = [0, 4, 1, 5, 2, 3];
    let sequential_dispatch = navigation_dispatch_samples(sequential_dispatch_photos, &sequential)?;
    let nonsequential_dispatch =
        navigation_dispatch_samples(nonsequential_dispatch_photos, &nonsequential)?;

    let mut warm_project = Project::new("Prefetched navigation benchmark");
    warm_project.photos = warm_photos.to_vec();
    warm_project.selected = warm_photos.first().map(|photo| photo.id);
    let mut warm_workspace = Workspace::new(warm_project, None);
    let warm_worker = PreviewWorker::new();
    let mut warm_pipeline = PreviewPipeline::default();
    let first = warm_photos
        .first()
        .context("warm navigation photos are empty")?;
    warm_pipeline.select(first.id, first.adjustments.clone());
    let mut prefetched_ready_samples = Vec::with_capacity(warm_photos.len() - 1);
    for photo in &warm_photos[1..] {
        let enqueue = warm_worker.request_prefetch(
            warm_pipeline.epoch(),
            photo.clone(),
            photo.adjustments.clone(),
        );
        anyhow::ensure!(
            enqueue.accepted.is_some(),
            "adjacent prefetch was not accepted"
        );
        warm_pipeline.track_enqueue(enqueue);
        let completion = warm_worker
            .recv_timeout(Duration::from_secs(5))
            .context("adjacent prefetch did not complete")?;
        anyhow::ensure!(
            matches!(
                warm_pipeline.complete(completion, Instant::now()),
                PreviewCompletionDisposition::Cached
            ),
            "adjacent prefetch did not enter the prepared cache"
        );

        let started = Instant::now();
        warm_workspace.execute(Command::Select { id: photo.id })?;
        warm_pipeline.select(photo.id, photo.adjustments.clone());
        let preview = warm_pipeline
            .take_cached(photo.id, &photo.adjustments)
            .context("selected adjacent preview was absent from the cache")?;
        consume_ready_preview(preview);
        prefetched_ready_samples.push(started.elapsed());
    }

    let mut project = Project::new("Non-sequential navigation benchmark");
    project.photos = cold_photos.to_vec();
    project.selected = cold_photos.first().map(|photo| photo.id);
    let mut workspace = Workspace::new(project, None);
    let worker = PreviewWorker::new();
    let mut pipeline = PreviewPipeline::default();
    let mut cold_ready_samples = Vec::with_capacity(nonsequential.len());
    for index in nonsequential {
        let started = Instant::now();
        let id = cold_photos[index].id;
        workspace.execute(Command::Select { id })?;
        let photo = workspace.project.photo(id)?.clone();
        let adjustments = photo.adjustments.clone();
        let generation = pipeline.select(id, adjustments.clone());
        anyhow::ensure!(
            pipeline.request_decision(Instant::now(), id, &adjustments)
                == PreviewRequestDecision::Request,
            "cold selected preview was unexpectedly cached, pending, or backed off"
        );
        let enqueue = worker.request_selected(generation, pipeline.epoch(), photo, adjustments);
        anyhow::ensure!(
            enqueue.accepted.is_some(),
            "cold selection was not accepted"
        );
        pipeline.track_enqueue(enqueue);
        let preview = wait_for_published_preview(&worker, &mut pipeline)?;
        consume_ready_preview(preview);
        cold_ready_samples.push(started.elapsed());
    }

    let mut large_prefetch = Photo::new(
        10_000,
        large_prefetch_path.to_owned(),
        "priority-prefetch.jpg".into(),
        6000,
        4000,
    );
    large_prefetch.format = "jpg".into();
    let selected = photos[24].clone();
    let priority_worker = PreviewWorker::new();
    let mut priority_pipeline = PreviewPipeline::default();
    priority_pipeline.select(selected.id, selected.adjustments.clone());
    let prefetch = priority_worker.request_prefetch(
        priority_pipeline.epoch(),
        large_prefetch,
        Adjustments::default(),
    );
    let prefetch_id = prefetch
        .accepted
        .as_ref()
        .context("priority prefetch was not accepted")?
        .id;
    priority_pipeline.track_enqueue(prefetch);
    wait_until_active(&priority_worker, prefetch_id)?;
    let priority_started = Instant::now();
    let generation = priority_pipeline.generation();
    let selected_enqueue = priority_worker.request_selected(
        generation,
        priority_pipeline.epoch(),
        selected.clone(),
        selected.adjustments.clone(),
    );
    let selected_id = selected_enqueue
        .accepted
        .as_ref()
        .context("priority selected preview was not accepted")?
        .id;
    priority_pipeline.track_enqueue(selected_enqueue);
    let first_completion = priority_worker
        .recv_timeout(Duration::from_secs(5))
        .context("priority selected preview did not complete")?;
    let selected_won = first_completion.identity.id == selected_id;
    let disposition = priority_pipeline.complete(first_completion, Instant::now());
    let priority_preview = match disposition {
        PreviewCompletionDisposition::Publish(preview) => *preview,
        PreviewCompletionDisposition::Cached => {
            wait_for_published_preview(&priority_worker, &mut priority_pipeline)?
        }
        PreviewCompletionDisposition::Failed(error) => {
            anyhow::bail!("priority selected preview failed: {error}")
        }
        PreviewCompletionDisposition::Ignored => {
            anyhow::bail!("priority completion was ignored")
        }
    };
    consume_ready_preview(priority_preview);
    let priority_duration = priority_started.elapsed();
    if selected_won {
        let prefetch_completion = priority_worker
            .recv_timeout(Duration::from_secs(5))
            .context("priority prefetch did not finish after selected publication")?;
        let _ = priority_pipeline.complete(prefetch_completion, Instant::now());
    }
    let priority_ms = duration_ms(priority_duration);
    let priority_pass = selected_won && priority_ms <= profile.cold_switch_ready_budget_ms();
    let priority_metric = json!({
        "name": "selected_over_prefetch_ready",
        "workload": "Selected 1800x1200 JPEG completes while an authoritative 24 MP prefetch is already decoding",
        "elapsed_ms": rounded(priority_ms),
        "selected_completed_first": selected_won,
        "target_ms": 75.0,
        "budget_ms": profile.cold_switch_ready_budget_ms(),
        "pass": priority_pass,
    });

    Ok(vec![
        latency_metric(
            "sequential_photo_switch_dispatch",
            "Adjacent core selection, pipeline generation, and accepted worker request",
            &sequential_dispatch,
            1.0,
            profile.switch_dispatch_budget_ms(),
        ),
        latency_metric(
            "nonsequential_photo_switch_dispatch",
            "Non-adjacent core selection, pipeline generation, and accepted worker request",
            &nonsequential_dispatch,
            1.0,
            profile.switch_dispatch_budget_ms(),
        ),
        latency_metric(
            "sequential_photo_switch_ready",
            "Distinct adjacent JPEG completes through PreviewWorker, cache insertion, selection, cache publication, and upload buffers",
            &prefetched_ready_samples,
            16.7,
            profile.prefetched_switch_ready_budget_ms(),
        ),
        latency_metric(
            "nonsequential_photo_switch_ready",
            "Distinct non-prefetched JPEG path through worker completion, generation-safe publication, histogram, and upload buffers",
            &cold_ready_samples,
            75.0,
            profile.cold_switch_ready_budget_ms(),
        ),
        priority_metric,
    ])
}

fn navigation_dispatch_samples(photos: &[Photo], order: &[usize]) -> Result<Vec<Duration>> {
    let mut project = Project::new("Navigation dispatch benchmark");
    project.photos = photos.to_vec();
    project.selected = photos.first().map(|photo| photo.id);
    let mut workspace = Workspace::new(project, None);
    let worker = PreviewWorker::new();
    let mut pipeline = PreviewPipeline::default();
    let mut samples = Vec::with_capacity(order.len());
    for index in order {
        let started = Instant::now();
        let id = photos[*index].id;
        workspace.execute(Command::Select { id })?;
        let photo = workspace.project.photo(id)?.clone();
        let adjustments = photo.adjustments.clone();
        let generation = pipeline.select(id, adjustments.clone());
        let enqueue = worker.request_selected(generation, pipeline.epoch(), photo, adjustments);
        anyhow::ensure!(
            enqueue.accepted.is_some(),
            "navigation dispatch was not accepted"
        );
        pipeline.track_enqueue(enqueue);
        samples.push(started.elapsed());
    }
    while pipeline.has_outstanding_work() {
        let completion = worker
            .recv_timeout(Duration::from_secs(5))
            .context("dispatch worker did not drain within five seconds")?;
        let _ = pipeline.complete(completion, Instant::now());
    }
    Ok(samples)
}

fn prepare_navigation_photos(directory: &std::path::Path) -> Result<Vec<Photo>> {
    let mut photos = Vec::with_capacity(25);
    for index in 0..25 {
        let (width, height) = if index < 12 { (640, 426) } else { (1800, 1200) };
        let path = directory.join(format!("navigation-{index:02}.jpg"));
        DynamicImage::ImageRgb8(deterministic_rgb_seeded(width, height, index as u32 + 1))
            .save(&path)
            .with_context(|| format!("prepare navigation source {}", path.display()))?;
        let mut photo = Photo::new(
            100 + index,
            path,
            format!("navigation-{index:02}.jpg"),
            width,
            height,
        );
        photo.format = "jpg".into();
        photos.push(photo);
    }
    Ok(photos)
}

fn wait_for_published_preview(
    worker: &PreviewWorker,
    pipeline: &mut PreviewPipeline,
) -> Result<PreparedPreview> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        anyhow::ensure!(!remaining.is_zero(), "selected preview timed out");
        let completion = worker
            .recv_timeout(remaining)
            .context("selected preview did not become ready within five seconds")?;
        match pipeline.complete(completion, Instant::now()) {
            PreviewCompletionDisposition::Publish(preview) => return Ok(*preview),
            PreviewCompletionDisposition::Failed(error) => {
                anyhow::bail!("selected preview failed: {error}")
            }
            PreviewCompletionDisposition::Cached | PreviewCompletionDisposition::Ignored => {}
        }
    }
}

fn wait_until_active(worker: &PreviewWorker, request_id: u64) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(1);
    while !worker.is_active(request_id) {
        anyhow::ensure!(
            Instant::now() < deadline,
            "prefetch did not become active within one second"
        );
        std::thread::yield_now();
    }
    Ok(())
}

fn consume_ready_preview(preview: PreparedPreview) {
    std::hint::black_box(preview.source.to_rgba8());
    std::hint::black_box(preview.rendered.to_rgba8());
    std::hint::black_box(preview.histogram);
}

fn deterministic_rgb(width: u32, height: u32) -> RgbImage {
    deterministic_rgb_seeded(width, height, 0)
}

fn deterministic_rgb_seeded(width: u32, height: u32, seed: u32) -> RgbImage {
    RgbImage::from_fn(width, height, |x, y| {
        Rgb([
            ((x * 13 + y * 3 + seed * 19) % 256) as u8,
            ((x * 5 + y * 11 + seed * 31) % 256) as u8,
            ((x * 7 + y * 17 + seed * 47) % 256) as u8,
        ])
    })
}

fn latency_metric(
    name: &str,
    workload: &str,
    samples: &[Duration],
    target_ms: f64,
    budget_ms: f64,
) -> serde_json::Value {
    let mut milliseconds: Vec<_> = samples.iter().copied().map(duration_ms).collect();
    milliseconds.sort_by(f64::total_cmp);
    let median = percentile(&milliseconds, 0.5);
    let p95 = percentile(&milliseconds, 0.95);
    json!({
        "name": name,
        "workload": workload,
        "samples": milliseconds.len(),
        "median_ms": rounded(median),
        "p95_ms": rounded(p95),
        "target_ms": target_ms,
        "budget_ms": budget_ms,
        "pass": p95 <= budget_ms,
    })
}

fn percentile(sorted: &[f64], quantile: f64) -> f64 {
    let index = ((sorted.len().saturating_sub(1)) as f64 * quantile).ceil() as usize;
    sorted[index]
}

fn duration_ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

fn rounded(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}
