# Spectrum preview

The GPUI application that will become Spectrum. The current editors remain in
`apps/spectrum`. Home and projects read and change the real Spectrum library through
the same engine as the `spectrum` CLI. Set `SPECTRUM_LIBRARY` to use a separate
library. The canvas editor is still a sample. Browser support is not a
requirement.

```sh
nix develop -c cargo run --release -p spectrum-demo --locked
```

The app opens to Home. Its Projects mode lists projects as covers; its Assets
mode shows the whole library with All assets, Unassigned, and Trash. Opening a
project scopes everything to it: the main area shows the project overview, an
image, or the sample canvas, and the sidebar shows one mode at a time. Modes are
capabilities, offered only when they apply: Assets, Color, and Layers. The strip
at the top of the sidebar shows the project and the mode buttons.

| Shortcut | Action |
| --- | --- |
| Command+1 to 5 | Switch to the nth sidebar mode |
| Command+K | Go to a project, place, or asset |
| Command+A, Esc | Select all assets, clear the selection |

Control replaces Command on Linux and Windows. Command+S is deliberately unbound.

Import assets accepts images and folders from a file picker or by dropping them
on the main area; inside a project, imports join it. Asset grids support click,
Shift-click, Command-click, and drag-box selection. Right-click acts on the whole
selection: rename, add to or remove from a project, delete, or restore. Opening
an image in a project shows Color, whose sliders make real engine edits. The
sample canvas shows Color acting on a selected layer. Canvases from the library
cannot be opened here yet.

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
