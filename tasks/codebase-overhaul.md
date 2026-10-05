---
status: in_progress
priority: high
---

# One-app codebase overhaul

Spectrum began as two apps, Lumen (photos) and Prism (canvases). The product
is now one app; this task removes the split from the code. There is no
backward compatibility to keep: no user work exists, and libraries may be
wiped. Images are their own asset (color edits); canvases compose image
assets; video will compose both.

1. Remove the egui apps, their crates, packaging, scripts, and tests.
2. Engines become `crates/spectrum-image` and `crates/spectrum-canvas`; the
   GPUI app becomes `apps/spectrum-desktop`.
3. Drop old file formats, version ladders, and migrations.
4. One asset model: a document per asset, owned by the library; one durable
   document layer for history and the live bridge; a CLI on assets only.
5. Shared engine crates for imaging, text, and compositing, for video next.
6. Desktop app: an editor per asset type over one save and history path.
7. Docs describe the result.

Later: a history tree view in the desktop app.
