# UI framework research

Research date: 2026-09-26. This is a source review, not a benchmark or an
approved framework change. See [workflow preferences](workflow.md).

## Browser preference

The user increasingly prefers the browser and terminal over installed apps.
They want us to consider a browser UI connected to a native Spectrum engine
running as a local daemon. WASM is a possible implementation, not a requirement.
Keep this option open before committing to the UI rewrite. Research comes before
the previously proposed GPUI controls page.

## Findings

Spectrum currently uses egui/eframe 0.35 with the glow OpenGL renderer, as declared
in the workspace Cargo.toml. Its recent performance fix did not require changing
frameworks. The user reports that the formerly slow interactions now work well.

[egui's README](https://github.com/emilk/egui#why-immediate-mode) explains that
immediate mode repeats layout during rendered frames. Large lists need visible-row
layout to avoid unnecessary work. Idle applications need not repaint continuously.
Its quoted 1-2 ms typical frame cost is an author estimate without a matched
Spectrum workload or hardware specification. It cannot establish a speed ratio.
Official eframe integrations support native and browser deployment, with glow or
wgpu rendering. Both egui and GPUI can use GPU acceleration.

[GPUI's README](https://github.com/zed-industries/zed/blob/main/crates/gpui/README.md)
describes a hybrid immediate/retained framework with declarative styled elements,
entities, keyboard actions, and custom rendering. macOS uses Metal. GPUI remains
pre-1.0 and warns about breaking changes. This makes it a plausible fit for our
custom sidebar and controls, with dependency maintenance to account for.

[Zed's 2024 performance report](https://zed.dev/blog/120fps) documents real frame
pacing and Metal synchronization work to achieve smooth 120 FPS on tested Macs.
It supports confidence in the team's performance engineering. It does not compare
GPUI with egui, establish browser performance, or measure creative media workloads.
No credible matched egui/GPUI benchmark was found in this research pass.

The egui team also builds a substantial application.
[Rerun's architecture](https://github.com/rerun-io/rerun/blob/main/ARCHITECTURE.md)
uses egui/eframe for both native and web viewers. Neither framework should be
judged solely by a toolkit demo's appearance.

## GPUI browser status

Upstream now contains [gpui_web](https://github.com/zed-industries/zed/tree/main/crates/gpui_web).
Its [module documentation](https://github.com/zed-industries/zed/blob/main/crates/gpui_web/src/gpui_web.rs)
specifies WebGPU with automatic WebGL2 fallback and one top-level canvas/window.
The crates.io API returned 404 for gpui_web during this review. Treat this as an
upstream implementation requiring evaluation, not established released parity
with native GPUI. Pin the source revision for any trial.

The [platform implementation](https://github.com/zed-industries/zed/blob/main/crates/gpui_web/src/platform.rs)
returns unsupported errors for native path prompts. Import/export needs browser
or daemon-specific handling. The upstream hello_web example uses nightly Rust
and cross-origin isolation headers. We have inspected source, not built or tested
this backend on the user's browser. Native Zed performance cannot answer that test.

## Possible Spectrum architecture

This is a proposal. A daemon on the user's Mac could serve the browser UI on
localhost, own the library and native processing, and expose engine commands,
state updates, and previews over authenticated HTTP/WebSocket connections.
Restrict the service to loopback and validate origins and sessions. The browser
would draw controls and the editing view. The CLI would retain the same engine
behavior. WASM can run Rust UI code, but a conventional web UI could also call
this service. A remote server engine is a separate choice, not implied here.

This requires a new browser-compatible protocol and client boundary. Existing
native engine dependencies and local IPC are not already a browser service.
Do not assume the entire current app compiles to WASM. Avoid blocking browser
input on processing requests. Coalesce slider updates and discard stale previews.
Preview transfer, decoding, and GPU uploads need measurement alongside UI frames.

## Recommendation and open decision

GPUI remains a reasonable native choice. egui has the more established Rust
browser path. If using Spectrum in the browser matters, evaluate GPUI's upstream
browser backend before a broad rewrite. The proposed controls page could test
both native and browser execution, including an image preview and interactions.
A conventional web UI remains an option if browser use becomes the priority.
The user has not approved an implementation or selected a final deployment model.
