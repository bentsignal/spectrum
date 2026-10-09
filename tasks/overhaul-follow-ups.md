---
status: todo
priority: normal
---

# Overhaul follow-ups

Work left after the [one-app overhaul](codebase-overhaul.md). Done since:
agents work in their own sessions (together or separate) and the desktop
follows a together agent live; HarfBuzz is the only text engine; large photos
render on canvases; the CLI ships inside the app executable; the
interaction benchmark and storage budgets guard editing speed.

1. **Remaining compatibility code.** The raster backing cache still inventories
   an older entry layout; gradients keep a legacy two-stop encoding; the
   revision store mirrors each session's default cursor in two tables.
2. **Desktop structure.** Split the library shell, palette, and picker out of
   `Workspace` the way the editors were, and stop reopening the library for
   every background task.
3. **Shared pieces for video.** Move text layout and fonts, the raster cache,
   and compositing out of `spectrum-canvas` as the video editor needs them.
4. **Zoom on large photos.** A zoom step takes about 0.85 s (software
   rendering) to come back sharp: each layer re-renders whole at the new
   size. Render only the visible part of a zoomed-in layer, then tighten the
   `canvas_zoom` budget in `apps/spectrum-desktop/src/benchmark.rs`.
5. **Placing a photo** takes about 0.8 s: the canvas embeds a full-resolution
   copy of the photo's render so its history replays exactly. A smaller
   stand-in would do, since the canvas always draws the photo's current render.

The history tree view is next, in [revision lifecycle](revision-lifecycle.md).
