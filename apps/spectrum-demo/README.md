# Spectrum controls preview

A separate GPUI application for reviewing Spectrum's next design. The current
editors remain in `apps/spectrum`. This app uses sample state only and never opens
or mutates the Spectrum library. Browser support is not a requirement.

```sh
nix develop -c cargo run --release -p spectrum-demo --locked
```

Review Controls, Adjustments, Assets, and Layers in the sidebar. Try typing,
opening the blend menu, dragging sliders, toggling switches and layer visibility,
selecting cards, and moving the sidebar to either edge. Use Reset demo to restore
the starting state. Command+Q on Mac or Control+Q quits.

## Control coverage

| Existing controls | Preview location |
| --- | --- |
| Project/import/export buttons and disabled actions | Controls |
| Names, checkboxes, switches, blend dropdown | Controls |
| Image adjustment sliders and reset | Adjustments |
| Library thumbnails, selection, empty state | Assets |
| Layer list, visibility, opacity, blend and fill color | Layers |
| Local/global editing choice | Layers |

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
