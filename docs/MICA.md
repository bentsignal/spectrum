# Mica

Mica is the suite's focused layered image editor: a small, fast, native Rust
application for the work that belongs on a canvas rather than in a photo
catalog. It complements Lumen instead of adding Photoshop-style complexity to
Lumen's development workspace.

The practical target is a streamlined Photoshop replacement for creating a
canvas, combining raster and text layers, transforming and cropping content,
masking or clipping layers, applying nondestructive adjustments, compositing,
undoing and redoing work, saving an editable project, and exporting a finished
image.

Editable documents use the `.mica` project extension. A project contains the
canvas and layer model needed to resume work; flattened exports are deliberately
separate outputs.

## One engine, two interfaces

Mica follows the same agent-first contract as Lumen:

```text
Native GUI (mica-gui) ─┐
                       ├─> Command -> Project -> compositor / shared imaging kernel
CLI (mica) ────────────┘
```

Every persistent GUI mutation is a typed core command. The `mica` CLI exposes
the same commands for people, scripts, and agents, while `mica-gui` provides a
fast native editing surface over that behavior. Project validation, range
checking, history, rendering, and persistence belong below both interfaces; a
GUI control is never the only way to perform an operation.

Use `mica schema` to discover the machine-facing command protocol and prefer the
task-oriented CLI subcommands for shell automation. Successful CLI calls emit
structured JSON so agents can inspect exact IDs and state rather than scraping
human UI text.

The global `--project <path>` option selects an editable `.mica` document.
Commands cover project creation and save, raster/text/shape layers, selection and
stack order, transforms, opacity and blend modes, visibility, masks and clipping,
per-layer adjustments, canvas crop/resize, history, export, and the Lumen
handoff. `schema`, raw `run`, and `benchmark` provide discovery, low-level agent
control, and repeatable performance checks.

## Relationship with Lumen

Lumen and Mica are separate applications with one-way reuse:

- Lumen owns cataloging, RAW development, culling, batch looks, and photo export.
- Mica owns canvases, layer stacks, transforms, masks, text, compositing, and
  document export.
- Mica depends on the shared Lumen imaging kernel for compatible development and
  color controls. Lumen does not depend on Mica.

The `from-lumen` handoff creates a layered Mica project from a developed Lumen
photo. It preserves the focused Lumen workflow, gives the new project a rendered
base layer to build on, and avoids a reverse package dependency. This boundary
also lets agents hand a selected catalog photo to Mica without reproducing
Lumen's RAW/development behavior inside the canvas editor.

Original photos remain immutable. Mica saves editing state into its project and
exports to a destination selected by the user; handing work across applications
does not overwrite the Lumen source.

The handoff is available without opening either GUI:

```sh
mica from-lumen \
  --catalog path/to/library.lumencatalog \
  --photo 42 \
  --output path/to/composition.mica
```

## Run and package

Run either interface from the workspace:

```sh
cargo run --release -p mica --bin mica -- schema
cargo run --release -p mica --bin mica-gui
```

Build an optimized package on its target operating system:

```sh
bash scripts/package-mica-macos.sh
bash scripts/package-mica-linux.sh
pwsh scripts/package-mica-windows.ps1
```

The scripts only build Cargo package `mica` and stage files beneath
`target/dist`; they do not modify a Lumen installation or project.
