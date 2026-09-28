# Spectrum preview

The GPUI application that will become Spectrum. The current editors remain in
`apps/spectrum`. Home and projects read and change the real Spectrum library through
the same engine as the `spectrum` CLI. Set `SPECTRUM_LIBRARY` to use a separate
library. Browser support is not a requirement.

```sh
nix develop -c cargo run --release -p spectrum-demo --locked
```

The app opens to Home, with Projects (covers and New project) and Assets (the
whole library with Import assets, All assets, Unassigned, and Trash) tabs.
Click selects projects and assets; double-click opens them. Opening a project
scopes everything to it. Its overview shows the project's
assets; opening an image or a canvas shows its editing modes, one at a time:
Color, Crop, and Info for images; Layers, Style, and Color for canvases. The back button at the
top of the sidebar goes up one level, from an item to the project, then Home.

| Shortcut | Action |
| --- | --- |
| Command+1 to 5 | Switch to the nth sidebar mode |
| Command+K | Go to a project, place, or asset |
| Command+A, Esc | Select all assets, clear the selection |
| Command+= and Command+- | Grid zoom, also the − and + buttons above grids |
| Command+Z, Command+Shift+Z | Undo and redo edits to the open image or canvas |
| Arrow keys, Shift+arrows | Nudge the selected canvas layer 1 or 10 pixels |
| Delete, Backspace | Remove the selected layer, or delete the selected assets |

Control replaces Command on Linux and Windows. Command+S is deliberately unbound.

Import assets accepts images and folders from a file picker or by dropping them
on the main area. Inside a project, Import assets also offers "From your
library…", a picker for assets already in Spectrum; imports join the project. Asset grids support click,
Shift-click, Command-click, and drag-box selection. Right-click acts on the whole
selection: rename, add to or remove from a project, delete, or restore. "Add to
project…" opens a searchable project picker that can also create a project for
the selection. Dragging a selection box near the top or bottom edge scrolls. Opening
an image in a project shows Color: a histogram above one section at a time
(Light, Color, Curves, Mixer, Grading, Detail). Each change sends the image's
whole adjustment model to the engine; Command+Z and Command+Shift+Z step its
edit history. Crop mode shows the whole frame with a draggable crop box and
aspect presets; its sidebar rotates, flips, and straightens. Compare in the title bar shows the unedited original beside the edit. Info
shows capture details, the image's projects, and the canvases that use it. Inside a project,
"Recently added" sorts by when assets joined it. Double-click a canvas, or use New canvas in a project, to open it. Canvases
offer three modes: Layers (add text, boxes, circles, and project images;
reorder, duplicate, delete, show and hide), Style (one section at a time:
Look for blending, opacity, text, and fills; Arrange for alignment and
rotation; Shadow for a drop shadow; or the canvas's size and background), and Color (the selected layer's adjustments;
image layers choose Edit locally or Edit globally). Click a layer on the canvas
to select it, drag to move it, or drag a corner handle to resize it. Image
layers can swap their image, which also replaces a missing-image placeholder.
Export… in the title bar or the right-click menu saves a full-size JPEG or
PNG. Delete or Backspace removes the selected layer or asks to delete the
selected assets. Command+Z steps canvas history too.

GPUI 0.2.2 and GPUI Component 0.5.1 are pinned. Spectrum owns its grayscale theme
in `src/theme.rs`. Component supplies text editing and common control behavior;
this is not a commitment to its default styling or every component it offers.
Custom canvas interaction, image processing, history, and project/import behavior
remain in the shared engines and are not part of this visual review.

CI builds only this desktop preview for distribution. Workspace lint/tests still
cover the existing engines and CLI. `scripts/package-spectrum-demo.sh` creates a
local package. On Mac it retains the existing signing/notarization flow, uses a
separate `com.bentsignal.spectrum.preview` bundle identity, and ships no old GUI
or CLI binaries. The outer bundle remains `Spectrum.app`; keep it in a separate
folder from the existing application while reviewing.
