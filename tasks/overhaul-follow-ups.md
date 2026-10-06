---
status: todo
priority: high
---

# Overhaul follow-ups

Work left after the [one-app overhaul](codebase-overhaul.md), roughly in order:

1. **Live agent edits in the desktop.** While an asset is open, notice when
   another session saves a newer revision and reload it, so CLI agents' edits
   appear as they land (the old live bridge did this for the egui apps).
2. **One text engine.** New desktop text still uses the character layout
   (`LegacyCharV1`); the CLI defaults to HarfBuzz. Switch the desktop to
   HarfBuzz, check font weights, tracking, and slider speed visually and with
   `spectrum canvas benchmark --strict`, then delete the character layout.
3. **Remaining compatibility code.** The raster backing cache still inventories
   an older entry layout; gradients keep a legacy two-stop encoding; the
   revision store mirrors each session's default cursor in two tables.
4. **Desktop structure.** Split the library shell, palette, and picker out of
   `Workspace` the way the editors were, and stop reopening the library for
   every background task.
5. **Shared pieces for video.** Move text layout and fonts, the raster cache,
   and compositing out of `spectrum-canvas` when the video editor needs them.

The history tree view is in [revision lifecycle](revision-lifecycle.md).
