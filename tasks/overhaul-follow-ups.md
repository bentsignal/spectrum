---
status: todo
priority: normal
---

# Overhaul follow-ups

Work left after the [one-app overhaul](codebase-overhaul.md). Done since:
agents work in their own sessions (together or separate) and the desktop
follows a together agent live; HarfBuzz is the only text engine; large photos
render on canvases; the CLI ships inside the app executable; the
interaction benchmark and storage budgets guard editing speed; edited photos
drag smoothly at any size; zoom, placing, and opening canvases render only
what is shown.

1. **Remaining compatibility code.** The raster backing cache still inventories
   an older entry layout; gradients keep a legacy two-stop encoding; the
   revision store mirrors each session's default cursor in two tables.
2. **Desktop structure.** Split the library shell, palette, and picker out of
   `Workspace` the way the editors were, and stop reopening the library for
   every background task.
3. **Shared pieces for video.** Move text layout and fonts, the raster cache,
   and compositing out of `spectrum-canvas` as the video editor needs them.
4. **RAW files over 25 megapixels** do not open in the image editor: their
   full development is refused above that size to bound memory (48 bytes a
   pixel). Develop the editor's 2560 px source at reduced size instead, then
   add a large RAW to the interaction benchmark.
5. **Old image renders.** Each set of edits on an image placed in a canvas
   leaves a full-size render in `previews/`; remove an image's older renders
   when a new one is made.
6. **Startup of large canvases** decodes every photo once per session. Keep
   the reduced copies on disk so a canvas reopened later starts from them.

The history tree view is next, in [revision lifecycle](revision-lifecycle.md).
