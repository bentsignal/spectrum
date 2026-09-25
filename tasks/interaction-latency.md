---
status: todo
priority: high
---

# Diagnose Spectrum interaction latency

On an Apple Silicon Mac, the user reports noticeable lag with one photo and a
simple text canvas: scrolling the photo sidebar, dragging develop sliders, and
dragging canvas text all feel slow. A catalog with roughly three shoots of 36
photos each makes the lag worse. The Mac is otherwise responsive. This report
is from the combined Spectrum app built in [CI run 36053949146](https://github.com/bentsignal/spectrum/actions/runs/36053949146).

Identify any work introduced by hosting the editors together and separate
frame scheduling or rendering cost from image development latency. Make only
targeted fixes justified by evidence because the current egui UI is planned
for a GPUI rebuild. Keep the existing controls and editing behavior intact.

## Acceptance criteria

- Record what local profiling and existing release benchmarks reveal.
- Fix a confirmed merge-specific bottleneck if one is found.
- Identify what still needs measurement on the user's Mac before claiming the
  subjective lag is resolved.

## Initial investigation

No runtime code changed in this pass. The source review found synchronous
thumbnail decoding and transient photo rendering on the UI thread. The unified
shell also polls the inactive editor, including collaboration and preview
completion work. These are candidates to measure, not confirmed causes of the
Mac report. The CI artifact is a native ARM macOS build.

On the NixOS server, `nix develop -c ./target/release/lumen benchmark --strict`
passed. The latest sample measured p95 transient adjustment preview at 3.50 ms,
tone-curve preview at 11.51 ms, and non-prefetched photo readiness at 56.43 ms.
These engine workloads do not measure displayed frame timing on the Mac.

`nix develop -c ./target/release/prism benchmark --strict` failed on four complex
rendering workloads: radial-gradient strips 113.025 ms against 100 ms,
angle-gradient viewport 32.261 ms against 30 ms, angle-gradient strips
143.621 ms against 100 ms, and gradient-shadow composition 646.312 ms against
500 ms. These failures do not establish why a simple text drag feels slow.

Next, capture frame timing on the Mac for sidebar scrolling, slider dragging,
and text dragging, including time in inactive-editor polling and preview work.
Use that evidence to choose a small fix or carry the requirement into the GPUI
implementation. The user explicitly wants to avoid spending heavily on the
current UI before replacing it. The reported lag remains unresolved.
