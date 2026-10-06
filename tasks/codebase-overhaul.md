---
status: done
priority: high
---

# One-app codebase overhaul

Spectrum began as two apps, Lumen (photos) and Prism (canvases). This removed
the split from the code, with no backward compatibility (libraries may be
wiped). Done, October 2026:

1. The egui apps, their crates, packaging, and scripts are gone.
2. Engines are `crates/spectrum-image` and `crates/spectrum-canvas`; the GPUI
   app is `apps/spectrum-desktop`; the library service is `crates/spectrum-assets`.
3. One storage format: no file-format migrations, version ladders, or
   compatibility readers; layer transfers have one version.
4. One asset model: each asset is one document in the library over the shared
   `spectrum-document` layer; the CLI works on assets only; the live bridge is gone.
5. Engines and shared pixel code sit in neutral crates. Text layout, the raster
   cache, and compositing stay in `spectrum-canvas` until video needs them.
6. Images and canvases save through one service path with stable sessions;
   the desktop's editor state is split per editor.
7. Docs describe the result: [Architecture](../docs/ARCHITECTURE.md).

What remains is in [overhaul follow-ups](overhaul-follow-ups.md).
