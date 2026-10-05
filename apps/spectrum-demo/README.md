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
| Command+1 to 5 | Switch to the nth sidebar mode (hold Command half a second to see the list beside the sidebar; Command+Up and Down step through it, and hovering a mode shows it) |
| Command+K | Go to a project, place, or asset; at Home, search the whole library |
| Command+A, Esc | Select all assets, clear the selection |
| Option+1 to 6 | Switch to the nth section of Color or Style |
| Command+P | On a canvas, choose a tool (Move, selection tools, Brush, Eraser, Text, Box, Circle, Gradient, Pen, Crop, Eyedropper, Image) |
| Command+A, Command+D, Command+Shift+I | On a canvas, select all, deselect, invert the selection |
| V M L W B E T U G P C I, X, D | On a canvas: Photoshop's tool keys (Shift+M, Shift+U cycle); swap and reset the colors |
| Command+E | Edit the selected text layer on the canvas |
| Command+C, Command+V | Copy the selected layer; paste it above the selection, in any canvas |
| Command+;, Command+Shift+; | Show or hide guides; turn snapping on or off |
| Scroll, Command+= and Command+−, Command+0, Command+Option+0 | Zoom the canvas about the pointer, step in and out, fit it, show it at 100% (sideways scroll, Command-scroll, Space-drag, or a middle-button drag pans) |
| Command+Z, Command+Shift+Z | Undo and redo edits to the open image or canvas |
| Command+Shift+C, Command+Shift+V | Copy the open or selected image's edits; paste them onto the open or selected images |
| Arrow keys, Shift+arrows | Nudge the selected canvas layer 1 or 10 pixels |
| Delete, Backspace | Hide what the selection covers on the layer under it (any kind), remove the selected layer, or delete the selected assets |

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
offer five modes, chosen from the sidebar's dropdown, each starting with the current tool: Overview (where it starts: the tool with its settings, the selected layer, the foreground and background colors with a swap arrow, and the selection's actions), Layers (the layer list: drag a row to reorder, with a line showing where it lands
(above or below the list means top or bottom); right-click to rename, duplicate, hide, put inside, or delete; layers inside another share a box with it), Style (one section at a time:
Look for blending, opacity, text (font, alignment, line height, tracking), and fills; Arrange for alignment and
rotation, and Guides (add, show, snapping); Effects lists Photoshop's layer styles with switches (drop shadow, stroke, glows, inner shadow, bevel & emboss, satin, color and gradient overlays) and shows the chosen one's settings below), Color (the selected layer's adjustments;
image layers choose Edit locally or Edit globally). Click a layer on the canvas
to select it, drag to move it, or drag a corner handle to resize it (that corner
stays put and the layer scales from the opposite side), or the handle above it to rotate it (snapping near 45°). Text places text where
you click; Box and Circle draw by dragging; Gradient drags a new full-canvas layer from the foreground to the background color; Move returns after. Overview holds the current tool, the colors, the tool's settings ([ and ] size the brush; Move's "Select layer on click" picks a layer's box first and what is inside it on a second click, or at once with Command; off, drags move the selected layer), and the selection's actions (fill, hide with Delete, crop, invert); dragging a box moves what is inside it. The Canvas mode sets the canvas's size (Resize… sets exact pixels, with proportions kept or not, or a preset), background, and guides. Marquee, Ellipse select, Lasso, and Magic wand select (Shift adds, Option subtracts, both intersect; a click off the canvas or Esc deselects); Delete hides what the selection covers on the selected layer, or else the topmost layer showing there, text and shapes included, which stay editable. The Brush previews its stroke exactly as it lands; the Eraser works on whatever layer shows where the stroke starts, of any kind, within the selection, and Look's mask group brings erased parts back; Pen clicks corners, drags curves, and keeps 45° with Shift; Crop drags the bounds to keep; Eyedropper shows a loupe with the color's code (⌘C copies). Every color picker has an eyedropper: click it, then the canvas, and the color lands in the picker, which stays open. Every slider's value can be clicked and typed. A slider drag saves, and undoes, as one step. Look's Inside and mask group puts a layer inside the one below it (it shows only where that layer is; dropping a row on another in the list does the same) or takes it out, and shows, inverts, or removes a layer's mask. Shape fills are Solid or Gradient, with a stop editor (drag, add, remove, reverse), kind, angle, scale, center, and size. With Snap on in the title bar, dragged layers snap to guides, the canvas, and other layers; drag a guide to move it, or off the canvas to remove it. Double-click text,
or press Command+E, to edit it in a field under it on the canvas. Edits apply
to a local copy of the document at once and save in order behind it; a dragged
layer, or one whose style changes, is drawn from separate renders of the layers
below it, the layer itself, and the layers above, so it updates at once. Image
layers can swap their image, which also replaces a missing-image placeholder.
Export… in the title bar or the right-click menu picks a format (JPEG, PNG,
TIFF, or WebP for images; JPEG or PNG for canvases), JPEG quality, and resolution
(100%, 75%, 50%, 25%, or a custom percent or width, showing the size it makes), then asks where to save; choices and the folder carry over. Delete or Backspace removes the selected layer or asks to delete the
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
typeface, and the field below it picks among a family's weights and styles. Hovering a font, or moving to it with the arrow keys, shows it on the
canvas at once; a click or Enter applies it (embedding the font in the canvas)
and Escape puts the old one back; scrolling previews whatever comes under the
pointer. Star a font to list it first (Favorites shows only those), hide it to clear
it away (Hidden shows them to bring back), and Latin only leaves out fonts
without English letters.
Fonts that do not allow embedding are marked.
