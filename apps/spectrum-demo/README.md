# Spectrum controls preview

A separate GPUI application for reviewing Spectrum's next design. The current
editors remain in `apps/spectrum`. This app uses sample state only and never opens
or mutates the Spectrum library. Browser support is not a requirement.

```sh
nix develop -c cargo run --release -p spectrum-demo --locked
```

The window is a mock workspace with sample assets. The sidebar holds the
controls for the current mode, and the main area shows the work. Switch modes
with the segmented control or Command+1, 2, and 3 (Control on Linux and
Windows). These shortcuts are trial bindings, not approved ones.

| Mode | Sidebar | Main area |
| --- | --- | --- |
| Library | Project menu, import, search, kind filters, sort, thumbnail size | Asset grid |
| Adjust | Light and color sliders, compare switch | Selected asset, optionally beside the original |
| Canvas | Layers, blend mode, opacity, local or global edits | Sample canvas |

The View menu switches between All assets, Unassigned, and projects;
"New project…" opens a dialog. Import adds a placeholder image to the current
project, or leaves it unassigned. Right-click an asset to add it to a project,
remove it from the current one, or delete it. Double-click an asset to open it in Adjust or Canvas. Adjustments carry
into the canvas that places the photo. The panel button at the top of the
sidebar moves it to the other edge.

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
