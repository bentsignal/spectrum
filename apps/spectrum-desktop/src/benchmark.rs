//! The interaction benchmark: `spectrum-desktop --benchmark [--strict]
//! [--profile interactive|software] [--report <file>] [--photo <file>]`.
//!
//! It makes a throwaway library holding a 24-megapixel photo (or `--photo`,
//! such as a camera RAW file) and a canvas,
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

mod fixtures;
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
    })
}

/// What the benchmark library holds.
#[derive(Clone, Debug)]
pub struct Fixtures {
    pub directory: PathBuf,
    pub project: ProjectId,
    pub photo: AssetId,
    pub canvas: AssetId,
    pub photo_layer: u64,
    pub text_layer: u64,
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
    fixtures::make(&directory, &library, options.photo.as_deref())
        .context("could not make the benchmark library")
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

/// Every budget, in milliseconds or counts. Before the overhaul, an image
/// edit reached the screen in 72 ms p50 and 139 ms p95 with software
/// rendering; these keep that bar and catch anything slower.
const BUDGETS: &[Budget] = &[
    budget("image_open", "elapsed", "max", 1000.0, 1500.0),
    budget("image_scrub", "image_edit_to_screen", "p95", 60.0, 120.0),
    budget("image_scrub", "image_edit_to_screen", "max", 150.0, 400.0),
    budget("image_scrub", "main_thread_lag", "p99", 20.0, 60.0),
    budget("image_scrub", "main_thread_lag", "max", 50.0, 150.0),
    // Saves run in the background beside the preview's renders, so a single
    // one can wait for a core; the typical save is held tight.
    budget("image_scrub", "image_save", "p50", 60.0, 80.0),
    budget("image_scrub", "image_save", "max", 1000.0, 1000.0),
    budget("image_scrub", "follow_check", "count", 2.0, 2.0),
    budget("image_scrub", "error_shown", "count", 0.0, 0.0),
    budget("canvas_open", "elapsed", "max", 2000.0, 3000.0),
    budget("canvas_open", "error_shown", "count", 0.0, 0.0),
    budget("canvas_drag", "main_thread_lag", "p99", 20.0, 60.0),
    budget("canvas_drag", "main_thread_lag", "max", 50.0, 150.0),
    budget("canvas_drag", "settle", "max", 300.0, 600.0),
    budget("canvas_drag", "error_shown", "count", 0.0, 0.0),
    budget("canvas_zoom", "settle", "p95", 800.0, 1200.0),
    budget("canvas_zoom", "main_thread_lag", "max", 60.0, 150.0),
    budget("canvas_zoom", "error_shown", "count", 0.0, 0.0),
    budget("text_size", "canvas_edit_to_screen", "p95", 60.0, 120.0),
    budget("text_size", "settle", "max", 300.0, 600.0),
    budget("text_size", "main_thread_lag", "p99", 20.0, 60.0),
    budget("text_size", "error_shown", "count", 0.0, 0.0),
    budget("brush_undo", "settle", "max", 300.0, 600.0),
    budget("brush_undo", "main_thread_lag", "max", 60.0, 150.0),
    budget("brush_undo", "error_shown", "count", 0.0, 0.0),
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
                        // Counts stay strict; times allow slower machines.
                        Profile::Ci if budget.stat == "count" => budget.software,
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
    let text = serde_json::to_string_pretty(&report).unwrap_or_default();
    println!("{text}");
    if let Some(path) = &options.report {
        let _ = std::fs::write(path, &text);
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
            if options.strict && !pass {
                FAILED.store(true, Ordering::Relaxed);
            }
            let _ = std::fs::remove_dir_all(&fixtures.directory);
            cx.update(|_, cx| cx.quit()).ok();
        })
        .detach();
    }
}
