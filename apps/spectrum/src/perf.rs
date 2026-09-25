//! Optional frame timing for diagnosing interaction lag on a user's machine.
//! The trace contains timings and input categories, never document contents.

use std::{
    fs::{File, OpenOptions},
    io::{BufWriter, Write},
    path::PathBuf,
    time::{Duration, Instant},
};

use eframe::egui;

use super::WorkspaceKind;

const TRACE_PATH_ENV: &str = "SPECTRUM_PERF_LOG";
const HEADER: &str = "frame,workspace,input,frame_gap_ms,documents_ms,switcher_ms,inactive_ms,active_ms,ui_ms,pixels_per_point\n";

pub(super) struct FrameTrace {
    output: BufWriter<File>,
    previous_frame: Option<Instant>,
    frame: u64,
}

pub(super) struct FrameSample {
    started: Instant,
    previous_gap: Option<Duration>,
    input: &'static str,
    pixels_per_point: f32,
    documents_done: Instant,
    switcher_done: Instant,
    inactive_done: Instant,
}

impl FrameTrace {
    pub(super) fn from_environment() -> Option<Self> {
        let path = std::env::var_os(TRACE_PATH_ENV).map(PathBuf::from)?;
        let file = match OpenOptions::new().create(true).append(true).open(&path) {
            Ok(file) => file,
            Err(error) => {
                eprintln!(
                    "Could not open Spectrum frame trace {}: {error}",
                    path.display()
                );
                return None;
            }
        };
        let needs_header = file
            .metadata()
            .map(|metadata| metadata.len() == 0)
            .unwrap_or(false);
        let mut output = BufWriter::new(file);
        if needs_header && output.write_all(HEADER.as_bytes()).is_err() {
            eprintln!(
                "Could not write Spectrum frame trace header to {}",
                path.display()
            );
            return None;
        }
        Some(Self {
            output,
            previous_frame: None,
            frame: 0,
        })
    }

    pub(super) fn start(&mut self, context: &egui::Context) -> FrameSample {
        let started = Instant::now();
        let previous_gap = self
            .previous_frame
            .map(|previous| started.duration_since(previous));
        self.previous_frame = Some(started);
        let input = context.input(|input| {
            if input.smooth_scroll_delta.length_sq() > 0.0 {
                "scroll"
            } else if input.pointer.any_down() && input.pointer.delta().length_sq() > 0.0 {
                "drag"
            } else if input.pointer.any_down() {
                "press"
            } else if input.pointer.delta().length_sq() > 0.0 {
                "move"
            } else {
                "idle"
            }
        });
        FrameSample {
            started,
            previous_gap,
            input,
            pixels_per_point: context.pixels_per_point(),
            documents_done: started,
            switcher_done: started,
            inactive_done: started,
        }
    }

    pub(super) fn finish(&mut self, sample: FrameSample, workspace: WorkspaceKind) {
        let finished = Instant::now();
        let _ = writeln!(
            self.output,
            "{},{},{},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{:.2}",
            self.frame,
            workspace.storage_value(),
            sample.input,
            sample.previous_gap.map(milliseconds).unwrap_or(0.0),
            milliseconds(sample.documents_done.duration_since(sample.started)),
            milliseconds(sample.switcher_done.duration_since(sample.documents_done)),
            milliseconds(sample.inactive_done.duration_since(sample.switcher_done)),
            milliseconds(finished.duration_since(sample.inactive_done)),
            milliseconds(finished.duration_since(sample.started)),
            sample.pixels_per_point,
        );
        self.frame += 1;
        if self.frame.is_multiple_of(60) {
            let _ = self.output.flush();
        }
    }
}

impl FrameSample {
    pub(super) fn documents_done(&mut self) {
        self.documents_done = Instant::now();
    }

    pub(super) fn switcher_done(&mut self) {
        self.switcher_done = Instant::now();
    }

    pub(super) fn inactive_done(&mut self) {
        self.inactive_done = Instant::now();
    }
}

fn milliseconds(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}
