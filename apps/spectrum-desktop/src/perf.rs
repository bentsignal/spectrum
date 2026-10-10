//! Interaction timing for the strict interaction benchmark.
//!
//! With `SPECTRUM_PERF_LOG=<file>` set, Spectrum measures what a person
//! feels: how late the main thread runs (any stall freezes input and
//! drawing) and how long an edit takes to reach the screen. It rewrites the
//! file with a JSON summary every half second. Without the variable nothing
//! is measured.
use gpui::*;
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

/// How often the main thread is asked to run; lateness beyond this is a stall.
const TICK: Duration = Duration::from_millis(4);

struct Log {
    path: PathBuf,
    samples: BTreeMap<&'static str, Vec<f64>>,
}

fn log() -> Option<&'static Mutex<Log>> {
    static LOG: OnceLock<Option<Mutex<Log>>> = OnceLock::new();
    LOG.get_or_init(|| {
        std::env::var_os("SPECTRUM_PERF_LOG").map(|path| {
            Mutex::new(Log {
                path: path.into(),
                samples: BTreeMap::new(),
            })
        })
    })
    .as_ref()
}

pub fn enabled() -> bool {
    log().is_some()
}

/// Records one measurement, in milliseconds, under `kind`.
pub fn record(kind: &'static str, elapsed: Duration) {
    if let Some(log) = log() {
        let mut log = log.lock().unwrap_or_else(|e| e.into_inner());
        log.samples
            .entry(kind)
            .or_default()
            .push(elapsed.as_secs_f64() * 1000.0);
    }
}

/// Measures main-thread lateness and writes the report while the app runs.
pub fn start(cx: &mut App) {
    if !enabled() {
        return;
    }
    cx.spawn(async move |cx| {
        let mut written = Instant::now();
        loop {
            let asked = Instant::now();
            cx.background_executor().timer(TICK).await;
            // This continuation runs on the main thread; anything blocking it
            // delays us exactly as it delays input and drawing.
            if cx.update(|_| ()).is_err() {
                break;
            }
            let lag = asked.elapsed().saturating_sub(TICK);
            record("main_thread_lag", lag);
            if written.elapsed() > Duration::from_millis(500) {
                write();
                written = Instant::now();
            }
        }
    })
    .detach();
}

/// Takes every measurement recorded so far, leaving none.
pub fn drain() -> BTreeMap<&'static str, Vec<f64>> {
    log().map_or_else(BTreeMap::new, |log| {
        std::mem::take(&mut log.lock().unwrap_or_else(|e| e.into_inner()).samples)
    })
}

fn write() {
    let Some(log) = log() else {
        return;
    };
    let log = log.lock().unwrap_or_else(|e| e.into_inner());
    let summary: BTreeMap<_, _> = log
        .samples
        .iter()
        .map(|(kind, samples)| (*kind, summarize(samples)))
        .collect();
    let temporary = log.path.with_extension("tmp");
    if std::fs::write(
        &temporary,
        serde_json::to_vec_pretty(&summary).unwrap_or_default(),
    )
    .is_ok()
    {
        let _ = std::fs::rename(temporary, &log.path);
    }
}

pub fn summarize(samples: &[f64]) -> serde_json::Value {
    let mut sorted = samples.to_vec();
    if sorted.is_empty() {
        return serde_json::json!({"count": 0});
    }
    sorted.sort_by(f64::total_cmp);
    let at = |q: f64| sorted[((sorted.len() - 1) as f64 * q).round() as usize];
    serde_json::json!({
        "count": sorted.len(),
        "p50": at(0.5),
        "p95": at(0.95),
        "p99": at(0.99),
        "max": sorted[sorted.len() - 1],
    })
}
