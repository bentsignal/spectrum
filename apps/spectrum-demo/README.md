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
| Command+K | Go to a project, place, or asset; at Home, search the whole library |
| Command+A, Esc | Select all assets, clear the selection |
| Option+1 to 6 | Switch to the nth section of Color or Style |
| Command+P | On a canvas, choose a tool (Move, selection tools, Brush, Eraser, Text, Box, Circle, Gradient, Pen, Crop, Image) |
| Command+A, Command+D, Command+Shift+I | On a canvas, select all, deselect, invert the selection |
| Command+E | Edit the selected text layer on the canvas |
| Command+C, Command+V | Copy the selected layer; paste it above the selection, in any canvas |
| Command+;, Command+Shift+; | Show or hide guides; turn snapping on or off |
| Command+Z, Command+Shift+Z | Undo and redo edits to the open image or canvas |
| Command+Shift+C, Command+Shift+V | Copy the open or selected image's edits; paste them onto the open or selected images |
| Arrow keys, Shift+arrows | Nudge the selected canvas layer 1 or 10 pixels |
| Delete, Backspace | Remove the selected layer, or delete the selected assets |

Control replaces Command on Linux and Windows. Command+S is deliberately unbound.

Import assets accepts images and folders from a file picker or by dropping them
on the main area. Inside a project, Import assets also offers "From your
library…", a picker for assets already in Spectrum; imports join the project. Asset grids support click,
Shift-click, Command-click, and drag-box selection. Right-click acts on the whole
selection: rename, add to or remove from a project, copy or paste edits, delete, or restore. "Add to
project…" opens a searchable project picker that can also create a project for
the selection. Dragging a selection box near the top or bottom edge scrolls. Opening
an image in a project shows Color: a histogram above one section at a time
(Light, Color, Curves, Mixer, Grading, Detail). Edits render in memory from a decoded copy of the image on every frame of a
drag and save once edits pause; Command+Z and Command+Shift+Z step its
edit history. Crop mode shows the whole frame with a draggable crop box and
aspect presets; its sidebar rotates, flips, and straightens. Compare in the title bar shows the unedited original beside the edit. Info
shows capture details, the image's projects, and the canvases that use it. Inside a project,
"Recently added" sorts by when assets joined it. Double-click a canvas, or use New canvas in a project, to open it. Canvases
offer three modes: Layers (the current tool, foreground and background colors,
and the layer list: drag a row to reorder, with a line showing where it lands
(above or below the list means top or bottom); duplicate, delete, show and hide), Style (one section at a time:
Look for blending, opacity, text (font, alignment, line height, tracking), and fills; Arrange for alignment and
rotation, and Guides (add, show, snapping); Effects for Photoshop's layer styles (drop shadow, stroke, glows, inner shadow, bevel & emboss, satin, color and gradient overlays; a style renders as a draft while its sliders move); or, with nothing selected, the canvas's size (Resize… sets exact pixels,
with proportions kept or not, or a preset) and background), and Color (the selected layer's adjustments;
image layers choose Edit locally or Edit globally). Click a layer on the canvas
to select it, drag to move it, or drag a corner handle to resize it (that corner
stays put and the layer scales from the opposite side). Text places text where
you click; Box and Circle draw by dragging; Gradient drags a new full-canvas layer from the foreground to the background color; Move returns after. Marquee, Ellipse select, Lasso, and Magic wand select (Shift adds, Option subtracts, both intersect) with marching ants; the Selection group fills, masks, hides (Delete), or crops to it. Brush and Eraser paint on Paint layers within the selection; Pen clicks out a filled shape (Enter or the first point closes it); Crop drags the new canvas bounds. Look's Mask and clipping group clips to the layer below and shows, inverts, or removes a layer's mask. Shape fills are Solid or Gradient, with a stop editor (drag, add, remove, reverse), kind, and angle. With Snap on in the title bar, dragged layers snap to guides, the canvas, and other layers; drag a guide to move it, or off the canvas to remove it. Double-click text,
or press Command+E, to edit it in a field under it on the canvas. Edits apply
to a local copy of the document at once and save in order behind it; a dragged
layer, or one whose style changes, is drawn from separate renders of the layers
below it, the layer itself, and the layers above, so it updates at once. Image
layers can swap their image, which also replaces a missing-image placeholder.
Export… in the title bar or the right-click menu picks a format (JPEG, PNG,
TIFF, or WebP for images; JPEG or PNG for canvases), JPEG quality, and full size
or a long edge, then asks where to save; choices and the folder carry over. Delete or Backspace removes the selected layer or asks to delete the
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

Colors open a full picker (a saturation and brightness field, a hue strip, hex
entry, and common colors). Beside Add in Layers, the foreground and background
colors sit beside the current tool; new layers take the foreground. The color
text shows hex, RGB, HSL, or OKLCH, chosen from a list in any picker and kept
for all of them, with copy and paste buttons. Canvas renders match the canvas's
size on screen, so layers stay sharp while they move.

The Font field lists the fonts installed on this computer, each in its own
typeface. Hovering a font, or moving to it with the arrow keys, shows it on the
canvas at once; a click or Enter applies it (embedding the font in the canvas)
and Escape puts the old one back. Fonts that do not allow embedding are marked.
