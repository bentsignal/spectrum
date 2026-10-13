//! The interaction benchmark: `spectrum-desktop --benchmark [--strict]
//! [--profile interactive|software|ci] [--report <file>] [--photo <file>]
//! [--footage <file>] [--only <scenario,...>]`.
//!
//! It makes a throwaway library holding a 24-megapixel photo (or `--photo`,
//! such as a camera RAW file), a 45-megapixel photo with a full editing
//! pass, canvases from one photo to twelve, and 4K 10-bit 4:2:2 HEVC
//! footage at 59.94 fps (or `--footage`, such as a camera's clip),
//! then drives the real app with synthetic mouse and keyboard input: the
//! same handlers a person's clicks and drags reach. It measures what a person
//! feels (how long an edit takes to reach the screen, how long the main
//! thread stalls, how long the canvas takes to settle) plus errors shown,
//! and fails under `--strict` when any budget is missed. `interactive`
//! budgets are for a computer with a GPU, `software` for software rendering
//! on a fast computer, and `ci` for shared CI machines.
use crate::workspace::Workspace;
use anyhow::{Context as _, Result};
use gpui::*;
use spectrum_library::{AssetId, ProjectId};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::atomic::{AtomicBool, Ordering},
};

mod features;
mod fixtures;
mod footage;
mod history;
mod image_zoom;
mod scenarios;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Profile {
    /// A computer with a GPU, as people use Spectrum.
    Interactive,
    /// Software rendering on a fast computer.
    Software,
    /// Shared CI machines: software rendering on few, slower cores.
    Ci,
}

#[derive(Clone, Debug)]
pub struct Options {
    pub strict: bool,
    pub profile: Profile,
    pub report: Option<PathBuf>,
    /// A photo to use instead of the generated one, such as a camera RAW.
    pub photo: Option<PathBuf>,
    /// Footage to use instead of the generated clip, such as a camera's.
    pub footage: Option<PathBuf>,
}

/// The benchmark's options, when the app was started with `--benchmark`.
pub fn options() -> Option<Options> {
    let args: Vec<String> = std::env::args().collect();
    if !args.iter().any(|arg| arg == "--benchmark") {
        return None;
    }
    let value = |flag: &str| {
        args.iter()
            .position(|arg| arg == flag)
            .and_then(|index| args.get(index + 1).cloned())
    };
    Some(Options {
        strict: args.iter().any(|arg| arg == "--strict"),
        profile: match value("--profile").as_deref() {
            Some("software") => Profile::Software,
            Some("ci") => Profile::Ci,
            _ => Profile::Interactive,
        },
        report: value("--report").map(PathBuf::from),
        photo: value("--photo").map(PathBuf::from),
        footage: value("--footage").map(PathBuf::from),
    })
}

/// The scenarios `--only a,b` names, for working on a few; a run with
/// `--only` checks only those and never stands for the whole benchmark.
fn only() -> Option<Vec<String>> {
    let args: Vec<String> = std::env::args().collect();
    let index = args.iter().position(|arg| arg == "--only")?;
    Some(args.get(index + 1)?.split(',').map(str::to_owned).collect())
}

/// What the benchmark library holds.
#[derive(Clone, Debug)]
pub struct Fixtures {
    pub directory: PathBuf,
    pub project: ProjectId,
    pub photo: AssetId,
    /// A larger photo with a full editing pass.
    pub edited_photo: AssetId,
    pub canvas: AssetId,
    pub photo_layer: u64,
    pub text_layer: u64,
    /// A 4K canvas of twelve photos, text with effects, and shapes.
    pub heavy_canvas: AssetId,
    /// A photo layer on it.
    pub heavy_layer: u64,
    /// A canvas drawn as one image: its shape blends with the photo.
    pub blend_canvas: AssetId,
    /// A poster: the photo clipped inside a frame under a title.
    pub poster_canvas: AssetId,
    /// The photo's layer on it.
    pub poster_layer: u64,
    /// A canvas every tool is tried on, and a shape layer on it.
    pub feature_canvas: AssetId,
    pub feature_layer: u64,
    /// A photo with a long branched history and an agent's session.
    pub history: history::HistoryFixture,
    /// 4K footage with sound.
    pub footage: AssetId,
    /// Ten seconds of stereo sound.
    pub song: AssetId,
}

/// Makes the benchmark library and points this process at it. Runs before
/// the app starts.
pub fn prepare(options: &Options) -> Result<Fixtures> {
    let directory = std::env::temp_dir().join(format!("spectrum-benchmark-{}", std::process::id()));
    if directory.exists() {
        std::fs::remove_dir_all(&directory)?;
    }
    std::fs::create_dir_all(&directory)?;
    let library = directory.join("library");
    // SAFETY: set before the app starts any thread.
    unsafe {
        std::env::set_var("SPECTRUM_LIBRARY", &library);
        // A first open decodes photos into an empty cache, as a person's
        // first open does.
        std::env::set_var("SPECTRUM_CACHE", directory.join("cache"));
        std::env::set_var("SPECTRUM_PERF_LOG", directory.join("perf.json"));
    }
    let fixtures = fixtures::make(
        &directory,
        &library,
        options.photo.as_deref(),
        options.footage.as_deref(),
    )
    .context("could not make the benchmark library")?;
    // The library's photos and renders are hundreds of megabytes just
    // written; flushing them now keeps that out of the first saves measured.
    #[cfg(unix)]
    std::process::Command::new("sync").status().ok();
    Ok(fixtures)
}

static FAILED: AtomicBool = AtomicBool::new(false);

/// Whether a strict benchmark missed a budget, for the exit code.
pub fn failed() -> bool {
    FAILED.load(Ordering::Relaxed)
}

/// A limit on one statistic of one measurement in one scenario.
struct Budget {
    scenario: &'static str,
    metric: &'static str,
    stat: &'static str,
    interactive: f64,
    software: f64,
}

const fn budget(
    scenario: &'static str,
    metric: &'static str,
    stat: &'static str,
    interactive: f64,
    software: f64,
) -> Budget {
    Budget {
        scenario,
        metric,
        stat,
        interactive,
        software,
    }
}

/// How much slower shared CI machines may be than the software profile.
const CI_SLOWDOWN: f64 = 3.0;

/// Every budget, in milliseconds or counts. An edit reaches the screen
/// within two or three frames, a drag never stalls the main thread, and
/// opening or zooming finishes in a fraction of a second, for a 24-megapixel
/// photo, a 45-megapixel one with every tab edited, and canvases from one
/// photo to twelve. Tighten these as Spectrum gets faster; never loosen one
/// to let a slower change in.
const BUDGETS: &[Budget] = &[
    budget("image_open", "elapsed", "max", 500.0, 600.0),
    budget("image_open", "main_thread_lag", "max", 60.0, 150.0),
    budget("image_scrub", "image_edit_to_screen", "p95", 40.0, 60.0),
    budget("image_scrub", "image_edit_to_screen", "max", 100.0, 200.0),
    budget("image_scrub", "main_thread_lag", "p99", 20.0, 60.0),
    budget("image_scrub", "main_thread_lag", "max", 50.0, 150.0),
    // Saves run in the background beside the preview's renders, so a single
    // one can wait for a core; the typical save is held tight.
    budget("image_scrub", "image_save", "p50", 60.0, 80.0),
    budget("image_scrub", "image_save", "max", 1000.0, 1000.0),
    // A drag's sharp frame shows soon after the drag pauses.
    budget("image_scrub", "image_sharp", "max", 200.0, 300.0),
    budget("image_scrub", "follow_check", "count", 2.0, 2.0),
    budget("image_scrub", "error_shown", "count", 0.0, 0.0),
    // A larger photo with every tab edited, as after a full editing pass.
    budget("edited_open", "elapsed", "max", 600.0, 700.0),
    budget("edited_open", "main_thread_lag", "max", 60.0, 150.0),
    budget("edited_scrub", "image_edit_to_screen", "p95", 40.0, 60.0),
    budget("edited_scrub", "image_edit_to_screen", "max", 100.0, 250.0),
    budget("edited_scrub", "main_thread_lag", "p99", 20.0, 60.0),
    budget("edited_scrub", "main_thread_lag", "max", 50.0, 150.0),
    budget("edited_scrub", "image_sharp", "max", 200.0, 300.0),
    budget("edited_scrub", "image_save", "p50", 60.0, 80.0),
    budget("edited_scrub", "image_save", "max", 1000.0, 1000.0),
    budget("edited_scrub", "error_shown", "count", 0.0, 0.0),
    budget("canvas_open", "elapsed", "max", 500.0, 600.0),
    budget("canvas_open", "main_thread_lag", "max", 60.0, 150.0),
    budget("canvas_open", "error_shown", "count", 0.0, 0.0),
    budget("canvas_drag", "main_thread_lag", "p99", 20.0, 60.0),
    budget("canvas_drag", "main_thread_lag", "max", 50.0, 150.0),
    budget("canvas_drag", "settle", "max", 200.0, 300.0),
    // The dragged layer's pixels stay under its outline, in points.
    budget("canvas_drag", "drag_lag", "max", 1.0, 1.0),
    budget("canvas_drag", "error_shown", "count", 0.0, 0.0),
    budget("canvas_zoom", "settle", "p95", 300.0, 600.0),
    budget("canvas_zoom", "main_thread_lag", "max", 60.0, 150.0),
    budget("canvas_zoom", "error_shown", "count", 0.0, 0.0),
    budget("text_size", "canvas_edit_to_screen", "p95", 40.0, 80.0),
    budget("text_size", "settle", "max", 250.0, 400.0),
    budget("text_size", "main_thread_lag", "p99", 20.0, 60.0),
    budget("text_size", "error_shown", "count", 0.0, 0.0),
    budget("brush_undo", "settle", "max", 150.0, 300.0),
    budget("brush_undo", "main_thread_lag", "max", 60.0, 150.0),
    budget("brush_undo", "error_shown", "count", 0.0, 0.0),
    // A stroke shows as it is painted: points not on screen yet.
    budget("brush_undo", "stroke_lag", "p95", 4.0, 6.0),
    // The same on a canvas drawn as one image.
    budget("poster_brush", "stroke_lag", "p95", 8.0, 12.0),
    budget("poster_brush", "settle", "max", 300.0, 450.0),
    budget("poster_brush", "main_thread_lag", "max", 60.0, 150.0),
    budget("poster_brush", "error_shown", "count", 0.0, 0.0),
    // A heavy canvas: twelve photos, text with effects, and shapes.
    budget("heavy_open", "elapsed", "max", 1200.0, 1500.0),
    budget("heavy_open", "main_thread_lag", "max", 60.0, 150.0),
    budget("heavy_open", "error_shown", "count", 0.0, 0.0),
    budget("heavy_drag", "main_thread_lag", "p99", 20.0, 60.0),
    budget("heavy_drag", "main_thread_lag", "max", 50.0, 150.0),
    budget("heavy_drag", "settle", "max", 300.0, 400.0),
    budget("heavy_drag", "drag_lag", "max", 1.0, 1.0),
    budget("heavy_drag", "error_shown", "count", 0.0, 0.0),
    budget("heavy_zoom", "settle", "p95", 300.0, 600.0),
    budget("heavy_zoom", "main_thread_lag", "max", 60.0, 150.0),
    budget("heavy_zoom", "error_shown", "count", 0.0, 0.0),
    // A canvas drawn as one image, as blend modes need.
    budget("blend_open", "elapsed", "max", 500.0, 600.0),
    budget("blend_open", "main_thread_lag", "max", 60.0, 150.0),
    budget("blend_open", "error_shown", "count", 0.0, 0.0),
    budget("blend_zoom", "settle", "p95", 300.0, 600.0),
    budget("blend_zoom", "main_thread_lag", "max", 60.0, 150.0),
    budget("blend_zoom", "error_shown", "count", 0.0, 0.0),
    // A poster: the photo, edited, clipped inside a frame under a title.
    budget("poster_open", "elapsed", "max", 500.0, 600.0),
    budget("poster_open", "main_thread_lag", "max", 60.0, 150.0),
    budget("poster_open", "error_shown", "count", 0.0, 0.0),
    budget("poster_drag", "drag_lag", "p95", 1.0, 1.0),
    // The first frames of a drag can trail a little while the layer's parts
    // are drawn apart; after that the layer stays under its outline.
    budget("poster_drag", "drag_lag", "max", 6.0, 6.0),
    budget("poster_drag", "main_thread_lag", "p99", 20.0, 60.0),
    budget("poster_drag", "main_thread_lag", "max", 50.0, 150.0),
    budget("poster_drag", "settle", "max", 200.0, 300.0),
    budget("poster_drag", "error_shown", "count", 0.0, 0.0),
    // The same zoomed in to 100%, where the poster renders at a capped size.
    budget("poster_zoom_drag", "drag_lag", "p95", 2.0, 4.0),
    // Zoomed in, a frame's move covers more of the screen, so the first
    // frames trail a little further.
    budget("poster_zoom_drag", "drag_lag", "max", 12.0, 16.0),
    budget("poster_zoom_drag", "main_thread_lag", "p99", 20.0, 60.0),
    budget("poster_zoom_drag", "main_thread_lag", "max", 50.0, 150.0),
    budget("poster_zoom_drag", "settle", "max", 300.0, 450.0),
    budget("poster_zoom_drag", "error_shown", "count", 0.0, 0.0),
    // The header's zoom from fit to 100% and back arrives sharp.
    budget("poster_zoom", "settle", "p95", 300.0, 600.0),
    budget("poster_zoom", "main_thread_lag", "max", 60.0, 150.0),
    budget("poster_zoom", "error_shown", "count", 0.0, 0.0),
    // The image editor's zoom to 100% and pan show sharp.
    budget("image_zoom", "settle", "p95", 400.0, 700.0),
    budget("image_zoom", "main_thread_lag", "max", 60.0, 150.0),
    budget("image_zoom", "error_shown", "count", 0.0, 0.0),
    // Every tool, layer control, and image slider works, is saved, and
    // undoes; each is drawn and saved quickly.
    // Each is timed from the release or click to drawn sharply, then to
    // saved, which waits for a 300 ms pause in editing by design.
    budget("canvas_tools", "feature_shown", "max", 300.0, 400.0),
    budget("canvas_tools", "feature_saved", "max", 700.0, 900.0),
    budget("canvas_tools", "main_thread_lag", "max", 60.0, 150.0),
    budget("canvas_tools", "error_shown", "count", 0.0, 0.0),
    budget("image_sliders", "feature_shown", "max", 250.0, 300.0),
    budget("image_sliders", "feature_saved", "max", 650.0, 700.0),
    budget("image_sliders", "main_thread_lag", "max", 60.0, 150.0),
    budget("image_sliders", "error_shown", "count", 0.0, 0.0),
    // The History mode on 1,200 revisions: open, previews, going to another
    // branch, and a watched agent's edit arriving; pans and zooms stay smooth.
    // Crop and Color switch without showing the other view's frame (a
    // problem fails the run); the full-size frame of the 45-megapixel,
    // fully edited photo then renders and the crop saves.
    budget("crop_switch", "settle", "max", 500.0, 650.0),
    budget("crop_switch", "main_thread_lag", "max", 60.0, 150.0),
    budget("crop_switch", "error_shown", "count", 0.0, 0.0),
    budget("history", "settle", "max", 300.0, 400.0),
    budget("history", "preview", "max", 500.0, 700.0),
    budget("history", "jump", "max", 300.0, 400.0),
    budget("history", "watch", "max", 400.0, 500.0),
    budget("history", "main_thread_lag", "max", 60.0, 150.0),
    budget("history", "error_shown", "count", 0.0, 0.0),
    // 4K 10-bit 4:2:2 HEVC at 59.94 fps, as a camera records it, readied
    // with its playback copy: the first frame shows promptly, a scrub's
    // picture follows the pointer and lands on its frame, and playback
    // (forward and backward) keeps every frame in sync with the sound.
    budget("footage_open", "elapsed", "max", 150.0, 200.0),
    budget("footage_open", "main_thread_lag", "max", 60.0, 150.0),
    budget("footage_open", "error_shown", "count", 0.0, 0.0),
    budget("footage_scrub", "player_frame_to_screen", "p95", 40.0, 60.0),
    budget(
        "footage_scrub",
        "player_frame_to_screen",
        "max",
        100.0,
        150.0,
    ),
    budget("footage_scrub", "caught_up", "max", 100.0, 150.0),
    // After the scrub stops, the original redraws the paused frame.
    budget("footage_scrub", "sharp", "max", 800.0, 1000.0),
    budget("footage_scrub", "main_thread_lag", "max", 60.0, 150.0),
    budget("footage_scrub", "error_shown", "count", 0.0, 0.0),
    budget("footage_play", "player_frames_behind", "p95", 1.0, 1.0),
    budget("footage_play", "player_catch_up", "count", 0.0, 0.0),
    budget("footage_play", "main_thread_lag", "p99", 20.0, 60.0),
    budget("footage_play", "main_thread_lag", "max", 50.0, 150.0),
    budget("footage_play", "error_shown", "count", 0.0, 0.0),
    // A song opens with its waveform drawn and plays.
    budget("audio_play", "elapsed", "max", 300.0, 400.0),
    budget("audio_play", "main_thread_lag", "max", 60.0, 150.0),
    budget("audio_play", "error_shown", "count", 0.0, 0.0),
];

/// One scenario's measurements.
pub struct Outcome {
    pub name: &'static str,
    pub samples: BTreeMap<&'static str, Vec<f64>>,
    /// A failure the scenario found itself, such as input with no effect.
    pub problem: Option<String>,
}

fn statistic(samples: Option<&Vec<f64>>, stat: &str) -> f64 {
    let Some(samples) = samples.filter(|samples| !samples.is_empty()) else {
        return 0.0;
    };
    if stat == "count" {
        return samples.len() as f64;
    }
    let mut sorted = samples.clone();
    sorted.sort_by(f64::total_cmp);
    let quantile = match stat {
        "p50" => 0.5,
        "p95" => 0.95,
        "p99" => 0.99,
        _ => 1.0,
    };
    sorted[((sorted.len() - 1) as f64 * quantile).round() as usize]
}

/// Checks every outcome against its budgets and prints the report.
fn report(outcomes: &[Outcome], options: &Options) -> bool {
    let mut failures = Vec::new();
    let scenarios: Vec<_> = outcomes
        .iter()
        .map(|outcome| {
            if let Some(problem) = &outcome.problem {
                failures.push(format!("{}: {problem}", outcome.name));
            }
            let checks: Vec<_> = BUDGETS
                .iter()
                .filter(|budget| budget.scenario == outcome.name)
                .map(|budget| {
                    let limit = match options.profile {
                        Profile::Interactive => budget.interactive,
                        Profile::Software => budget.software,
                        // Counts, and how far a dragged layer usually trails,
                        // stay strict; times, and the first frames of a drag
                        // (they trail while its parts render), allow slower
                        // machines.
                        Profile::Ci
                            if budget.stat == "count"
                                || (budget.metric == "drag_lag" && budget.stat != "max") =>
                        {
                            budget.software
                        }
                        Profile::Ci => budget.software * CI_SLOWDOWN,
                    };
                    let value = statistic(outcome.samples.get(budget.metric), budget.stat);
                    let pass = value <= limit;
                    if !pass {
                        failures.push(format!(
                            "{} {} {} = {value:.1} (budget {limit})",
                            outcome.name, budget.metric, budget.stat
                        ));
                    }
                    serde_json::json!({
                        "metric": budget.metric, "stat": budget.stat,
                        "value": (value * 10.0).round() / 10.0, "budget": limit, "pass": pass,
                    })
                })
                .collect();
            let metrics: BTreeMap<_, _> = outcome
                .samples
                .iter()
                .map(|(kind, samples)| (*kind, crate::perf::summarize(samples)))
                .collect();
            serde_json::json!({
                "scenario": outcome.name,
                "problem": outcome.problem,
                "checks": checks,
                "metrics": metrics,
            })
        })
        .collect();
    let pass = failures.is_empty();
    let report = serde_json::json!({
        "ok": true,
        "pass": pass,
        "profile": format!("{:?}", options.profile).to_lowercase(),
        "failures": failures,
        "scenarios": scenarios,
    });
    // The terminal gets a short summary; the full report goes to a file,
    // the Desktop unless `--report` names one, to send on as it is.
    let text = serde_json::to_string_pretty(&report).unwrap_or_default();
    let path = options.report.clone().unwrap_or_else(|| {
        let desktop = std::env::var_os("HOME").map(|home| PathBuf::from(home).join("Desktop"));
        desktop
            .filter(|desktop| desktop.is_dir())
            .unwrap_or_else(std::env::temp_dir)
            .join("spectrum-benchmark.json")
    });
    println!(
        "{}",
        if pass {
            "Benchmark passed."
        } else {
            "Benchmark failed:"
        }
    );
    for failure in &failures {
        println!("  {failure}");
    }
    match std::fs::write(&path, &text) {
        Ok(()) => println!("Full report: {}", path.display()),
        Err(error) => println!("Could not write the report to {}: {error}", path.display()),
    }
    pass
}

impl Workspace {
    /// Runs every scenario, reports, and quits.
    pub fn run_benchmark(
        &mut self,
        fixtures: Fixtures,
        options: Options,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.spawn_in(window, async move |this, cx| {
            let outcomes = scenarios::run_all(&this, cx, &fixtures).await;
            let pass = report(&outcomes, &options);
            let _ = std::fs::remove_dir_all(&fixtures.directory);
            if options.strict && !pass {
                FAILED.store(true, Ordering::Relaxed);
                // Quitting on macOS ends the process with success before
                // `main` can return, so a failed run exits here.
                std::process::exit(1);
            }
            cx.update(|_, cx| cx.quit()).ok();
        })
        .detach();
    }
}
