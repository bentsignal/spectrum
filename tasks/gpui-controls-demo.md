---
status: done
priority: high
---

# Build the GPUI controls review app

Build the approved native GPUI demo. See [design direction](../docs/design/workflow.md)
and [demo usage](../apps/spectrum-demo/README.md). Browser support is optional.

Acceptance: interactive controls for image/canvas UI review, sidebar on either
side, sample asset and layer views, and a signed/notarized Mac download. CI should
package the demo instead of the old desktop app. Retain engine/CLI quality checks.
Do not connect mock controls to library mutations. Project/import workflows follow
visual approval. Record verification here after the complete required loop passes.

Verification: complete fmt/clippy/workspace-test loop passed. Linux release package
and automated Xvfb/lavapipe interactions passed, including text, dropdowns, slider
dragging, visibility, scope, sidebar switching, resizing, and quit. All CI jobs passed.
Build cleanup reclaimed 7 GiB of superseded debug binaries; target is 26 GiB.
[CI and downloads](https://github.com/bentsignal/spectrum/actions/runs/36278142798).
Mac signing, notarization, stapling, Gatekeeper, and archive contents verified.
