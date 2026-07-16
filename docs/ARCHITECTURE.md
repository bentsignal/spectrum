# Architecture

Lumen has one behavior path and two interfaces:

```text
Native GUI (lumen-gui) ─┐
                        ├─> Command -> Workspace -> Project / Render engine
CLI (lumen) ────────────┘
```

The desktop app does not own a second editing model. Buttons and committed
slider changes produce values from `lumen_core::Command`. The CLI produces those
same values directly or deserializes them through `lumen run '<json>'`.

## Crate layout

- `src/adjustments.rs`: serializable nondestructive settings and sparse patches
- `src/project.rs`: catalog model, image metadata, atomic sidecar persistence
- `src/engine.rs`: pure-Rust transforms, color pipeline, and image encoding
- `src/command.rs`: the complete mutation boundary, clipboard, and undo/redo
- `src/bin/lumen.rs`: structured automation interface
- `src/bin/lumen-gui.rs`: native egui/eframe interface

The library is named `lumen_core`; both binaries link it in-process. There is no
daemon, local socket, embedded browser, or network requirement.

## Catalog guarantees

- Imported photos are referenced by canonical path and never overwritten.
- Every edit is stored as settings in a readable versioned JSON document.
- Saving writes a temporary sibling before replacing the catalog.
- A multi-file import is transactional in memory: if one file is invalid, none
  of that command's files are added.
- Adjustment values are sanitized inside the core, not only in the UI.

## Rendering

Preview and export use the same `render_photo` function. Previews set a long-edge
limit; exports default to source resolution. The current pipeline performs, in
order:

1. optional long-edge downsample (never upscale)
2. rotation and flips
3. temperature, tint, and exposure
4. shadow/highlight and black/white tone shaping
5. contrast and clarity
6. vibrance and saturation
7. vignette

The renderer currently operates on 8-bit RGBA. A future high-bit-depth/RAW path
should keep the `Command`, `Project`, and `Adjustments` API stable while replacing
the internal working buffer and adding input color profiles.

## Cross-platform choices

- egui/eframe with the lightweight OpenGL backend for the native UI
- `image` with only JPEG, PNG, TIFF, and WebP codecs enabled
- `rfd` for operating-system file dialogs
- no application database, async runtime, telemetry, or update service
- thin LTO, one codegen unit, symbol stripping, and abort-on-panic in release
