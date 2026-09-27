# Spectrum preview

The GPUI application that will become Spectrum. The current editors remain in
`apps/spectrum`. Library mode reads and changes the real Spectrum library through
the same engine as the `spectrum` CLI. Set `SPECTRUM_LIBRARY` to use a separate
library. Adjust and Canvas still show sample content. Browser support is not a
requirement.

```sh
nix develop -c cargo run --release -p spectrum-demo --locked
```

The sidebar holds the controls for the current mode, and the main area shows the
work. Switch modes with the segmented control or Command+1, 2, and 3 (Control on
Linux and Windows). These shortcuts are trial bindings, not approved ones.

| Mode | Sidebar | Main area |
| --- | --- | --- |
| Library | Import, search, view, sort, kind toggles, thumbnail size | Your library |
| Adjust | Light and color sliders, compare switch | Sample image, optionally beside the original |
| Canvas | Layers, blend mode, opacity, local or global edits | Sample canvas |

Import opens a file picker that accepts images and folders; dropping files on the
main area also imports them. Imports go into the project being viewed, or stay
unassigned. Thumbnails render in the background. The view button opens a search
palette over the content area for All assets, Unassigned, Trash, and projects;
typing a new name offers to create that project. Right-click an asset to add it
to a project, remove it from the current one, delete it, or restore it from the
trash. Delete asks for confirmation and links to the asset's projects.

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
