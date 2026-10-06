---
status: todo
priority: normal
---

# Overhaul follow-ups

Work left after the [one-app overhaul](codebase-overhaul.md). Done since:
agents work in their own sessions (together or separate) and the desktop
follows a together agent live; HarfBuzz is the only text engine; large photos
render on canvases; the CLI ships inside the app executable.

1. **Remaining compatibility code.** The raster backing cache still inventories
   an older entry layout; gradients keep a legacy two-stop encoding; the
   revision store mirrors each session's default cursor in two tables.
2. **Desktop structure.** Split the library shell, palette, and picker out of
   `Workspace` the way the editors were, and stop reopening the library for
   every background task.
3. **Shared pieces for video.** Move text layout and fonts, the raster cache,
   and compositing out of `spectrum-canvas` as the video editor needs them.
4. **Large photos on canvases.** They render by splitting into tiles that each
   decode part of the full-resolution source; a reduced-resolution level would
   make scaled-down views cheaper.

The history tree view is next, in [revision lifecycle](revision-lifecycle.md).
