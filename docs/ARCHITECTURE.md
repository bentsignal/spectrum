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

- `crates/spectrum-imaging`: app-neutral adjustments and pixel rendering shared
  by Spectrum tools
- `apps/lumen/src/project.rs`: catalog model, persistent edit history, and atomic
  sidecar persistence
- `apps/lumen/src/engine.rs`: Lumen-specific RAW decoding and export adapters
- `apps/lumen/src/command.rs`: the complete mutation boundary, clipboard, and
  undo/redo
- `apps/lumen/src/bin/lumen_cli/`: structured automation interface modules
- `apps/lumen/src/bin/lumen_gui/`: focused native GUI modules for state, library,
  toolbar, inspector, canvas, dialogs, and drawing helpers

The Lumen library remains named `lumen_core`; both binaries link it in-process.
`lumen_core` re-exports the shared Spectrum adjustment types for catalog API
compatibility. There is no daemon, local socket, embedded browser, or network
requirement.

## Catalog guarantees

- Imported photos are referenced by canonical path and never overwritten.
- Every edit is stored as settings in a readable versioned JSON document.
- Saving writes a temporary sibling before replacing the catalog.
- A multi-file import is transactional in memory: if one file is invalid, none
  of that command's files are added.
- Adjustment values are sanitized inside the core, not only in the UI.
- Every committed edit stores a complete snapshot and cursor in catalog v5.
- Camera/lens metadata and the unmarked/keep/reject culling state live beside each
  immutable source reference. Older RAW catalogs populate missing metadata lazily.
- Imports form lightweight chronological shoot batches. Existing catalogs migrate
  into batches using capture dates, and the Library renders them left-to-right.
  Batches retain their first/last capture dates plus the local catalog-import date,
  which provides a stable timeline label when camera metadata is unavailable.
- Sources underneath the catalog directory serialize as relative paths, allowing an
  iCloud/shared library folder to move between devices without path repair.
- Catalog-level presets store development settings while intentionally excluding crop,
  rotation, flips, and straighten so one look can be reused across different framing.
- Reset is an ordinary history event, so stepping backward restores prior work.

## Rendering

Preview and export use the same `render_photo` function. Previews set a long-edge
limit; exports default to source resolution. The current pipeline performs, in
order:

1. Sony ARW demosaic, white balance, camera calibration, and sRGB conversion
2. optional long-edge downsample (never upscale)
3. rotation, flips, filled straighten, and normalized crop
4. optional chroma-preserving noise reduction
5. temperature, tint, exposure, and tonal shaping
6. contrast, texture, clarity, and dehaze
7. eight-band HSL mixing, global saturation/vibrance, and three-way color grading
8. master and per-channel point curves, vignette, sharpening, and repair-brush dabs

RAW development starts in a 16-bit intermediate; the interactive adjustment
pipeline currently operates on 8-bit RGBA after sRGB conversion. A future
high-bit-depth working buffer can keep the command and catalog APIs stable.

Import preparation is parallel and reads RAW dimensions and EXIF metadata without
demosaicing the full sensor image. Interactive RAW previews prefer the camera's
embedded preview, then cache the decoded 1800px source instead of developing a RAW
again for every control movement. Full-resolution exports still develop the RAW
sensor data. While a pointer drag is active, the GUI renders
a 960px working preview and resolves the full cached preview on release. Pixel rows
are processed in parallel, and identity color, HSL, and curve stages are skipped.
This keeps the export path deterministic while avoiding work during interaction.

The repeatable release benchmark for this path is:

```sh
cargo test --release -p spectrum-imaging interactive_preview_benchmark -- --ignored --nocapture
```

For end-to-end budgets, `lumen benchmark --strict` also measures tone-curve
command persistence, a deterministic 12-photo JPEG import, and a deterministic
24 MP JPEG export. An optional `--raw-import PATH` sample measures real RAW
metadata import on a machine with an accessible camera file. Linux CI runs that
command against the optimized binary so material regressions block the build.
The CI invocation uses the documented `hosted-ci` budget profile because shared
two-core runners are not representative of an editing workstation.

## Cross-platform choices

- egui/eframe with the lightweight OpenGL backend for native composition
- `image` with only JPEG, PNG, TIFF, and WebP codecs enabled
- `rfd` for operating-system file dialogs
- no application database, async runtime, telemetry, or update service
- thin LTO, one codegen unit, symbol stripping, and abort-on-panic in release
