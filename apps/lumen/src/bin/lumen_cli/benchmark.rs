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
        let navigation_photos: Vec<_> = (0..6)
            .map(|index| {
                let mut photo = Photo::new(
                    100 + index,
                    import_source.clone(),
                    format!("switch-{index}.jpg"),
                    2400,
                    1600,
                );
                photo.format = "jpg".into();
                photo
            })
            .collect();
        let navigation_metrics = navigation_switch_metrics(&navigation_photos, profile)?;

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
    profile: BenchmarkProfile,
) -> Result<Vec<serde_json::Value>> {
    let sequential: Vec<_> = (0..photos.len()).collect();
    let nonsequential = [0, 4, 1, 5, 2, 3];
    let sequential_dispatch = navigation_dispatch_samples(photos, &sequential)?;
    let nonsequential_dispatch = navigation_dispatch_samples(photos, &nonsequential)?;

    let prepared: Vec<_> = photos
        .iter()
        .map(|photo| prepare_preview(photo, photo.adjustments.clone()))
        .collect::<Result<_>>()?;
    let mut selection = PreviewSelection::default();
    let mut prefetched_ready_samples = Vec::with_capacity(prepared.len());
    for preview in &prepared {
        let started = Instant::now();
        selection.select(preview.photo_id, preview.adjustments.clone());
        let completion = PreviewCompletion {
            request: PreviewRequest::Prefetch {
                epoch: selection.epoch(),
            },
            photo_id: preview.photo_id,
            adjustments: preview.adjustments.clone(),
            result: Err("prepared benchmark frame is held by the cache".into()),
        };
        anyhow::ensure!(
            selection.can_publish(&completion),
            "prefetched navigation result was rejected"
        );
        std::hint::black_box(preview.source.to_rgba8());
        std::hint::black_box(preview.rendered.to_rgba8());
        std::hint::black_box(&preview.histogram);
        prefetched_ready_samples.push(started.elapsed());
    }

    let mut project = Project::new("Non-sequential navigation benchmark");
    project.photos = photos.to_vec();
    project.selected = photos.first().map(|photo| photo.id);
    let mut workspace = Workspace::new(project, None);
    let worker = PreviewWorker::new();
    let mut selection = PreviewSelection::default();
    let mut cold_ready_samples = Vec::with_capacity(nonsequential.len());
    for index in nonsequential {
        let started = Instant::now();
        let id = photos[index].id;
        workspace.execute(Command::Select { id })?;
        let photo = workspace.project.photo(id)?.clone();
        let adjustments = photo.adjustments.clone();
        let generation = selection.select(id, adjustments.clone());
        worker.request_selected(generation, selection.epoch(), photo, adjustments);
        let completion = worker
            .recv_timeout(Duration::from_secs(5))
            .context("selected preview did not become ready within five seconds")?;
        anyhow::ensure!(
            selection.can_publish(&completion),
            "selected navigation result was stale"
        );
        let preview = completion
            .result
            .map_err(anyhow::Error::msg)
            .context("prepare selected navigation preview")?;
        std::hint::black_box(preview.source.to_rgba8());
        std::hint::black_box(preview.rendered.to_rgba8());
        std::hint::black_box(preview.histogram);
        cold_ready_samples.push(started.elapsed());
    }

    Ok(vec![
        latency_metric(
            "sequential_photo_switch_dispatch",
            "Adjacent core selection and asynchronous preview request; UI keeps the prior atomic frame",
            &sequential_dispatch,
            1.0,
            profile.switch_dispatch_budget_ms(),
        ),
        latency_metric(
            "nonsequential_photo_switch_dispatch",
            "Non-adjacent core selection and asynchronous preview request; UI keeps the prior atomic frame",
            &nonsequential_dispatch,
            1.0,
            profile.switch_dispatch_budget_ms(),
        ),
        latency_metric(
            "sequential_photo_switch_ready",
            "Prefetched adjacent 1800px preview: generation validation plus original/developed texture buffers and histogram",
            &prefetched_ready_samples,
            16.7,
            profile.prefetched_switch_ready_budget_ms(),
        ),
        latency_metric(
            "nonsequential_photo_switch_ready",
            "Non-prefetched 2400x1600 JPEG selection through background decode/develop/histogram and texture buffers",
            &cold_ready_samples,
            75.0,
            profile.cold_switch_ready_budget_ms(),
        ),
    ])
}

fn navigation_dispatch_samples(photos: &[Photo], order: &[usize]) -> Result<Vec<Duration>> {
    let mut project = Project::new("Navigation dispatch benchmark");
    project.photos = photos.to_vec();
    project.selected = photos.first().map(|photo| photo.id);
    let mut workspace = Workspace::new(project, None);
    let worker = PreviewWorker::new();
    let mut selection = PreviewSelection::default();
    let mut samples = Vec::with_capacity(order.len());
    for index in order {
        let started = Instant::now();
        let id = photos[*index].id;
        workspace.execute(Command::Select { id })?;
        let photo = workspace.project.photo(id)?.clone();
        let adjustments = photo.adjustments.clone();
        let generation = selection.select(id, adjustments.clone());
        worker.request_selected(generation, selection.epoch(), photo, adjustments);
        samples.push(started.elapsed());
    }
    Ok(samples)
}

fn deterministic_rgb(width: u32, height: u32) -> RgbImage {
    RgbImage::from_fn(width, height, |x, y| {
        Rgb([
            ((x * 13 + y * 3) % 256) as u8,
            ((x * 5 + y * 11) % 256) as u8,
            ((x * 7 + y * 17) % 256) as u8,
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
