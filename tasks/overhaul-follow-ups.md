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
7. **Zooming into an edited photo on a canvas** waits for the photo's
   full-size render the first time after each edit (about 0.5 s for 24 MP,
   longer for larger ones). Render the part on screen from the original with
   its edits instead (`render_image_region_at_source_resolution`).
8. **macOS main-thread stall on opening a heavy canvas** fails the macOS
   interaction job: 737 ms and 984 ms on the CI runner (a virtual machine),
   under 60 ms on Linux. Spectrum's own main-thread handlers (`main_*` in the
   report) take under 5 ms, so the time is inside GPUI's drawing; a stall of
   about one second matches `CAMetalLayer.nextDrawable` waiting for a busy
   GPU (GPUI's `metal_renderer.rs`). Check it on a real Mac with
   `spectrum-desktop --benchmark --strict --only heavy_open`.

The history tree view is next, in [revision lifecycle](revision-lifecycle.md).
