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

## Durable documents and sessions

`spectrum-document` turns any `Model` (a document type plus its commands) into
a durable document. Its history is an immutable tree of revisions, like Git
commits; nothing is ever rewritten or discarded. Snapshots are stored every 100
commands or 64 KiB of commands, so opening replays a bounded tail. Files a
document uses (source photos, fonts, Clone Stamp sources) are embedded by
content hash as `spectrum-asset:<sha256>.<ext>` and staged into the cache.

Every person and agent moves through the tree in their own session. The
desktop always commits onto the revision it shows, so editing from an older
revision, or beside an agent's work, makes a new branch. An agent starts from
where the person is and commits in its own session; it never moves the person.
Two ways to work with an agent (`spectrum-assets/src/sessions.rs`):

- **Together** (the CLI's default): the person follows the agent's revisions,
  and the open asset updates as they land, until the person makes an edit of
  their own. That edit branches; the agent's next command starts again from it.
- **Separate** (`spectrum agent start --mode separate`, then `--session`): the
  person never moves. They can jump to the agent's revisions later.

`Model` hooks let an engine resolve state-dependent commands before storing them
(a Clone Stamp source, a magic wand's selection, the selection a stroke paints
within) so replay is exact. There is one storage format; others are refused.

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
zoom and export scale. Text shapes with HarfBuzz (`spectrum-fonts`) and
renders cached glyph outlines; fonts are embedded in the document.

Interactive previews and exports share one CPU compositor, which renders only
the visible region, splits regions whose sources exceed one staging pass (such
as a large photo shown scaled down), and keeps decoded rasters in a bounded cache. A linked image
layer carries its image's asset ID; the service points it at the image's current
render, so image edits show on every canvas that uses it.

## Desktop app

`apps/spectrum-desktop` is a GPUI app. `Workspace` holds the library shell
(Home, projects, grid, palette, picker) plus one state struct per editor:
`ImageEditor` (color and crop controls, the open image's preview and saves) and
`CanvasControls` with the open `CanvasState`. The canvas editor applies commands
to its document immediately and saves them in batches after a 300 ms pause;
the image editor renders in memory and saves the whole adjustment set when
edits pause. Both save through the service onto the revision on screen, off
the main thread, and follow an agent the person works together with while
nothing is waiting to save. The desktop executable is also the CLI: started as
`spectrum` (packages link that name to it), it runs the command line.

## Performance checks

Three layers keep interaction fast, and all run in CI:

- `bash scripts/interaction-benchmark.sh` runs the desktop app itself
  (`spectrum-desktop --benchmark`) on a throwaway library with a 24-megapixel
  photo (or `--photo <file>`, such as a camera RAW). It drives a slider scrub,
  canvas open, layer drag, zoom steps, a text size drag, and brush strokes with
  undo through the app's handlers, and fails when an edit is slow to reach the
  screen, the main thread stalls, a canvas is slow to settle, or an error shows.
  Budgets are per profile: `interactive` (a GPU), `software` (Linux with
  software rendering), and `ci`. With `SPECTRUM_PERF_LOG=<file>` an ordinary
  session writes the same measurements.
- `crates/spectrum-assets/tests/interaction_costs.rs` bounds the storage work
  (document opens, disk flushes, writes, re-hashed bytes) of each open, save,
  idle check, and read, counted by `spectrum_document::io_stats`, so costs that
  are slow only on some systems fail everywhere.
- `spectrum images benchmark --strict` and `spectrum canvas benchmark --strict`
  bound engine operations.

On screen, raster layers shown at half their size or less draw from a cached
reduced copy, large PNG sources are decoded once into a backing file, and the
app prepares each source once per session; exports always read sources exactly.
