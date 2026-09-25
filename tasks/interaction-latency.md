---
status: done
priority: high
---

# Diagnose Spectrum interaction latency

On an Apple Silicon Mac, the user reports lag during photo import, sidebar
scrolling, slider and zoom changes, canvas text entry, and text dragging, even
with one photo and one text layer. A larger catalog worsens it. The original
report used [CI run 36053949146](https://github.com/bentsignal/spectrum/actions/runs/36053949146).
Preserve the current UI and prioritize targeted fixes because GPUI is planned.

## Acceptance criteria

- Record what local profiling and existing release benchmarks reveal.
- Fix a confirmed merge-specific bottleneck if one is found.
- Identify what still needs measurement on the user's Mac before claiming the
  subjective lag is resolved.

## Prior investigation and trace mechanism

Source review found synchronous thumbnail and transient photo rendering, plus
polling of the inactive editor. Earlier Lumen engine benchmarks passed, while
Prism's interactive-workstation profile missed four complex-render budgets;
neither result diagnosed simple interaction lag. `SPECTRUM_PERF_LOG` enables
a CSV of input category, frame gap, and UI phases without project contents.
The gap includes rendering and scheduling, which the UI phase times exclude.
The Mac artifact is a native ARM build. The user's trace is now analyzed below.

## September 2026 Mac trace and targeted fix

The user's 1,514-frame Apple Silicon trace shows median UI time of 36.74 ms
and median frame start gap of 39.96 ms, about 25 frames per second. Photo
drag frames have median UI time of 42.41 ms; photo scroll frames 36.53 ms;
canvas drag frames 35.15 ms. Even while Canvas is active, polling the hidden
Photos workspace takes about 23 ms per drag frame. While Photos is active,
polling the hidden Canvas workspace takes about 12 ms. This is enough to
explain broad interaction lag without blaming the photo and canvas engines.

Both editors called `DiscoveryLease::refresh` twice per UI frame. That
atomically wrote and synced the live-discovery record to disk even when the
event range was unchanged. The lease already has a background timer for TTL
renewal. Each editor now publishes the event range only when it changes;
failed publications are retried. Controls, document behavior, and UI layout
were not changed.

On this NixOS machine, an empty-app trace's median UI time fell from 9.23 ms
to 0.38 ms after the change (hidden workspace polling from 3.42 ms to
0.17 ms). This before/after sample ran on the local desktop session and is
evidence for reduced local UI-thread work, not a Mac frame-rate claim. A
separate automated Xvfb session opened a ten-photo catalog and injected
pointer and sidebar scroll input: 1,200 recorded frames, including 294
scroll frames and a Canvas switch, had median UI time 0.51 ms and median
hidden-workspace time 0.05 ms. Its first catalog frame took 365 ms. This
provides a repeatable Linux interaction check, but it does not reproduce
Mac hardware, Retina scale, or every widget path.
The reusable check is `scripts/spectrum-interaction-smoke-linux.sh`; run it
inside `nix develop -c nix shell nixpkgs#xdotool -c` with a catalog path.

The Mac trace has two adjacent photo frames lasting 4.69 and 1.35 seconds.
The first may include the native file picker, which runs inside the UI call;
the trace has no event marker that separates picker wait from actual import.
The 10-photo core import completed in about 121 ms locally, while the strict
Lumen batch-import benchmark passed at p95 4.51 ms for its smaller synthetic
fixture. Neither result measures the user's Mac import from picker to ready
thumbnails. Canvas text taking seconds to appear is likewise not explained by
the recorded UI durations, which do not measure async work completion.

The complete format, Clippy, and workspace test loop passed after the fix.
The Linux package script succeeded, as did both affected strict release
benchmarks (Prism with the hosted-CI profile). The user then tested the Mac
build from commit `1752ce4` and reported that zoom, scrolling, sliders,
and canvas text dragging now feel fast, with the overall lag essentially
gone. This closes the reported interaction-latency issue; later regressions
can be tracked
separately with the Linux smoke script and Mac trace.
