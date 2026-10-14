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
                                     ├─ spectrum-media    footage and audio: FFmpeg decoding, playback copies, export
                                     ├─ spectrum-video    video asset: tracks, clips, speed, compositing
                                     └─ spectrum-document durable documents: history, files, sessions
                                            └─ spectrum-revisions  revision store (SQLite)
shared pixels: spectrum-imaging · shaping/fonts: spectrum-fonts · media time: spectrum-time
```

Rust sources stay at or under 1,000 lines; `tools/workspace-guardrails` checks
this, task files, and the documentation budget.

## Assets and the library

`spectrum-library` is authoritative. It records each asset's UUID, kind
(`AssetKind`: image, canvas, video, audio, music), name, and document; projects
that group assets without owning them; import batches; typed references (a
canvas uses images; video will use everything); and a 30-day trash. Canvases
that use a trashed or purged image draw a same-sized placeholder.

Every asset is one `.spectrum` document inside the library: `images/<uuid>.spectrum`,
`canvases/<uuid>.spectrum`, `footage/<uuid>.spectrum`, and `audio/<uuid>.spectrum`.
Imported video and sound files are too large to embed, so they live once in
the media store, `media/<sha256>.<ext>`, copied in (and hashed) on import; the
index records which assets use which file, and a file goes when its last
asset is purged. `media/.cache/` holds rebuildable playback copies. `previews/` and each folder's `.cache/` are
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

A branch runs from the revision that began it along each revision's oldest
later one (`History::branch_of`); later ones begin branches of their own.
Branch names go on the revision that began the branch, in the document's
metadata (`revision_name:<id>`, `spectrum-revisions/src/store/names.rs`):
naming is not an edit and moves no one. The desktop's History mode
(`history_view.rs`, `history_layout.rs`, `history_paint.rs`,
`history_panel.rs`) draws the tree a column per step and a row per branch,
so lines never cross; only what is on screen is drawn, with previews
(`Service::revision_preview`, cached under `previews/`) once zoomed in.
Clicking a revision goes there, as does choosing a branch in the branch
finder (the palette, ⌘P, in History); Back and Forward retrace the
revisions visited. Watching a session moves the person along with it as the
document's file changes, until the person edits or moves on their own.

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

## Media engine

`spectrum-media` reads every common container and codec through FFmpeg
(`ffmpeg-next`; development links the system's FFmpeg, and macOS and Linux
packages build its LGPL configuration into the binary). `probe` reads a file's
picture, exact frame rate, frame count, rotation, and sound tracks;
`FrameReader` decodes the frame showing at any time, seeking to the keyframe
before it and decoding on, and converts on several threads to RGBA or BGRA;
`AudioReader` decodes a track as 48 kHz stereo. Times are `spectrum_time::Time`,
counted in flicks (1/705,600,000 s), which land exactly on every common frame
and sample rate. Footage larger than HD or deeper than 8 bits gets a
display-sized MPEG-2 playback copy with short groups of pictures and the same
frames at the same times, so scrubbing, playing backward, and future speed
ramps decode thousands of frames a second.

Decoding uses VideoToolbox on macOS, Direct3D 11 on Windows, and VA-API on
Linux when a device opens, falling back to software per stream
(`SPECTRUM_HARDWARE_DECODING=off` turns it off). `Writer` exports H.264 or
HEVC (x264 and x265 where FFmpeg has them, the platform's encoder otherwise)
and ProRes 422 HQ, with AAC sound.

## Video engine

`spectrum-video` holds the video asset: video tracks (the first at the
bottom) and audio tracks of clips that use footage, audio, images, and
canvases. Its commands overwrite or insert, move, trim (with or without
ripple), roll, slip, slide, cut, and delete, with linked picture and sound
changing together; each applies all or nothing, and new clips go on the
lowest free track. A clip's speed is velocity keyframes or a time remap
(`speed.rs`, integrated exactly per curve), anchored by an origin so trims
and cuts never move its frames; footage that runs out repeats. `Freeze`
holds a frame and plays on; `Reverse` mirrors the speed about the clip's
last frame, so it plays the same frames backward; `ConvertSpeed` keeps a
clip's timing as the other form within a quarter frame. `render`
composites a frame (top-down, stopping at the first opaque full-frame clip)
and mixes sound through providers; `spectrum-assets::video_render` supplies
them from playback copies or originals. Slow motion between source frames
blends, or follows each pixel's motion (`spectrum-imaging::flow`, pyramidal
dense flow with occlusion-aware drawing) for paused frames and exports. Sound
follows the speed by resampling, unless a clip keeps it.

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

The video editor (`video_*.rs`) applies each timeline edit on screen at once
and saves batches after a pause, as the canvas does; its program monitor is
the same player fed by the video's composited frames and mixed sound. Tools
and commands are in the tool button, Command+P (`video_menu.rs`), and the
timeline's right-click menu; tabs are Clip, Speed, Media, Export, History.
`video_layout.rs` lays out the grip, ruler, video tracks (anchored at the
divider), and audio tracks, each pane scrolling and clipped. Speed shows as
velocity or time remap from one timing (`video_speed.rs`); editing in the
other view converts first. Preview renders (`video_preview.rs`) keep JPEG
frames keyed by what makes each frame, so an edit loses only its frames.
Clip thumbnails and waveforms wait while the program plays. The
player (`player*.rs`) shows footage and audio. A picture thread decodes
from the playback copy (ahead of the playhead while playing, newest request
first while scrubbing), and a paused frame then redraws from the original. The
sound thread owns the output device; samples played are the playback clock,
and the picture skips ahead rather than drift if it falls behind. Copies are
made in the background, one at a time at low priority, paused while playing.

## Performance checks

Three layers keep interaction fast, and all run in CI:

- `bash scripts/interaction-benchmark.sh` runs the desktop app itself
  (`spectrum-desktop --benchmark`) on a throwaway library: a 24-megapixel
  photo (or `--photo <file>`, such as a camera RAW), a 45-megapixel photo with
  every tab edited, and canvases of one photo, twelve photos with text effects
  (one clipped in a frame), a blend mode, and a poster, 4K 59.94 fps 10-bit
  4:2:2 HEVC footage at a camera's bitrate (or `--footage <file>`), and a
  song. It drives slider scrubs, opens, layer drags, zoom steps and header
  zooms, image zoom and pan, a text size drag, brush strokes with undo, and
  footage scrubs, playback, and the transport keys through the app's handlers,
  and fails when an edit is slow to reach the screen, the main thread stalls,
  a dragged layer trails its outline, a canvas or image is slow to come back
  sharp, or an error shows. It also uses every canvas tool, the layer
  controls, and every image slider on a feature canvas and the photo, and
  fails when one does nothing, differs on disk from what is shown, will not
  undo and redo, or is slow to be drawn and saved. Budgets are per profile:
  `interactive` (a GPU), `software` (Linux with software rendering), and `ci`.
  `--only <scenario,...>` runs a few while working on them. With
  `SPECTRUM_PERF_LOG=<file>` an ordinary session writes the same measurements.
  The terminal shows pass or fail and where the full report went: the
  Desktop, or `--report <file>`.
- `crates/spectrum-assets/tests/interaction_costs.rs` bounds the storage work
  (document opens, disk flushes, writes, re-hashed bytes) of each open, save,
  idle check, and read, counted by `spectrum_document::io_stats`, so costs that
  are slow only on some systems fail everywhere.
- `spectrum images benchmark --strict` and `spectrum canvas benchmark --strict`
  bound engine operations.

What keeps it fast:

- **Image editor.** The open photo is decoded once at 2560 px (and at two
  smaller sizes). Geometry and noise reduction are kept between frames, so a
  slider frame redoes only color, spots, and sharpening, on every core. A drag
  shows the largest size that renders within a 60 Hz frame on this computer,
  and the full-size frame follows as soon as the drag pauses.
- **Canvas.** A layer renders whole only up to 4 megapixels; zoomed in
  further, only the part on screen renders sharp over it, and layers off
  screen keep their images. Canvases drawn as one image (blend modes) do the
  same for the whole canvas. Photos decode into memory once per session
  (within 1 GiB) and draw from reduced copies at small sizes; nothing is
  written to disk to show a canvas. An image edited since its canvas last
  showed it draws from a 2048 px stand-in while its full render is made in
  the background.
- **Compositing.** Canvases drawn as one image render in bands of rows on
  every core. Shapes, paths, and paint draw only the part each band needs,
  so a full-page shape zoomed in on a Retina screen never draws whole; text
  draws whole once and is kept between bands and renders;
  shadows and vector masks give the same pixels however a render is split.
  `SPECTRUM_BENCHMARK_SCREEN=2880x1800 GPUI_X11_SCALE_FACTOR=2` runs the
  Linux benchmark at a Retina screen's scale.
- **Drags and zooms.** A layer moving inside another, or on a canvas drawn as
  one image, draws apart from the rest so it stays under its outline; the
  exact render follows the drop. Zooms from the keys or the header move over
  140 ms while the new size renders. In the image editor, zoomed past the
  preview's pixels, the part on screen renders from the full-resolution photo.
  A two-finger gesture pans or zooms by how it starts, until the fingers lift.
- **Background work** (thumbnails) runs one item at a time and only while a
  grid shows it. Exports always read sources exactly, through the verified
  raster cache, which never flushes to disk.

