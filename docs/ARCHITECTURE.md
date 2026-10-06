# Architecture

Spectrum is one native Rust app. Every creative asset lives in an
app-managed library, and one command engine per asset kind serves both the
desktop app and the `spectrum` CLI.

```text
apps/spectrum-desktop (GPUI) ─┐
                              ├─> spectrum-assets (library service)
apps/spectrum (CLI) ──────────┘      ├─ spectrum-library  index: identity, kind, name, projects, references, trash
                                     ├─ spectrum-image    image asset: source photo + adjustments
                                     ├─ spectrum-canvas   canvas asset: layers, compositing, text, paint
                                     └─ spectrum-document durable documents: history, files, sessions
                                            └─ spectrum-revisions  revision store (SQLite)
shared pixels: spectrum-imaging · shaping/fonts: spectrum-fonts
```

Rust sources stay at or under 1,000 lines; `tools/workspace-guardrails` checks
this, task files, and the documentation budget.

## Assets and the library

`spectrum-library` is authoritative. It records each asset's UUID, kind
(`AssetKind`: image, canvas, video, audio, music), name, and document; projects
that group assets without owning them; import batches; typed references (a
canvas uses images; video will use everything); and a 30-day trash. Canvases
that use a trashed or purged image draw a same-sized placeholder.

Every asset is one `.spectrum` document inside the library: `images/<uuid>.spectrum`
and `canvases/<uuid>.spectrum`. `previews/` and each folder's `.cache/` are
disposable. `SPECTRUM_LIBRARY` or `--library` selects another library; the
default is `Library` in Spectrum's application-data directory. Back up by
copying the whole library directory while Spectrum is closed.

`spectrum-assets::Service` is the only way apps change the library. It imports
(one image document per file), creates canvases, edits, renames, copies (a
canvas copy gets its own copies of its images), places images on canvases,
renders previews and thumbnails, exports outside the library, and trashes,
restores, and purges. Names live only in the index.

## Durable documents

`spectrum-document` turns any `Model` (a document type plus its commands) into
a durable document. Each edit is a revision with its actor and session; undo and
redo move a session's cursor through an immutable tree, so editing from the
past keeps the old future as a branch. Snapshots are stored every 100 commands
or 64 KiB of commands, so opening replays a bounded tail. Files a document uses
(source photos, fonts, Clone Stamp sources) are embedded by content hash as
`spectrum-asset:<sha256>.<ext>` and staged into the cache on demand.

The desktop opens a library as the person ("You"); the CLI opens it as an agent.
Each has one lasting session per library. Edits open the document at its newest
revision, so a CLI edit builds on what the desktop saved and the reverse.
There is one storage format; files from other formats are refused.

`Model` hooks let an engine resolve state-dependent commands before storing them
(a Clone Stamp source, a magic wand's selection, the selection a stroke paints
within) so replay is exact, and validate documents after loading.

## Image engine

`spectrum-image` holds the image document (`Image`: source, size, camera
metadata, `Adjustments`) and its commands: `Adjust`, `SetAdjustments`, `Reset`,
`Undo`, `Redo`. Crop, rotation, flips, straighten, curves, HSL, grading, and
spot repair are all adjustments. `engine` decodes JPEG, PNG, TIFF, WebP, and
Sony ARW (via rawler) and renders previews and exports through one path:

1. RAW demosaic, white balance, camera calibration, and sRGB conversion
2. rotation, flips, filled straighten, and normalized crop
3. optional long-edge downsample after geometry (never upscale)
4. noise reduction; temperature, tint, exposure, and tonal shaping
5. contrast, texture, clarity, and dehaze
6. HSL mixing, saturation and vibrance, and three-way color grading
7. point curves, vignette, sharpening, and repair dabs

RAW development is bounded to 25 megapixels for previews; exports develop at
full resolution. The adjustment pipeline itself lives in `spectrum-imaging`,
which canvases also use for per-layer adjustments.

## Canvas engine

`spectrum-canvas` owns the layered document: raster, text, rectangle, ellipse,
path, and paint layers; transforms; pixel and vector masks; clipping; 27 blend
modes including Dissolve; layer styles and gradient fills; selections (rectangle,
ellipse, lasso, magic wand); brush, eraser, and Clone Stamp strokes; guides,
snapping, and alignment; and layer copy and paste. Every change is a
`spectrum_canvas::Command`. Shapes stay parametric and re-render at the current
zoom and export scale. Text shapes with HarfBuzz (`spectrum-fonts`) or the
older character layout; fonts are embedded in the document.

Interactive previews and exports share one CPU compositor, which renders only
the visible region and keeps decoded rasters in a bounded cache. A linked image
layer carries its image's asset ID; the service points it at the image's current
render, so image edits show on every canvas that uses it.

## Desktop app

`apps/spectrum-desktop` is a GPUI app. `Workspace` holds the library shell
(Home, projects, grid, palette, picker) plus one state struct per editor:
`ImageEditor` (color and crop controls, the open image's preview and saves) and
`CanvasControls` with the open `CanvasState`. The canvas editor applies commands
to its document immediately and saves them in batches after a 300 ms pause;
the image editor renders in memory and saves the whole adjustment set when
edits pause. Both save through the service, off the main thread.

## Performance checks

```sh
spectrum images benchmark --strict
spectrum canvas benchmark --strict
cargo test --release -p spectrum-imaging interactive_preview_benchmark -- --ignored --nocapture
```

`--profile hosted-ci` relaxes budgets for shared CI runners. Run the matching
strict benchmark for rendering or interaction changes.
