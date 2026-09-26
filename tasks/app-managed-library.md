---
status: in_progress
priority: high
---

# Complete Spectrum's app-managed creative library

The user confirmed the linked image/canvas flow works on Mac. Spectrum remains
greenfield; old names and formats carry no compatibility obligations. See
[direction](../docs/DIRECTION.md) and [library architecture](../docs/SUITE.md).

## Implemented

- [x] Stable editable asset UUIDs, extensible types, typed references, transitive
  dependency traversal, and cycle rejection in the neutral library crate.
- [x] App-managed image and canvas documents, immutable originals, existing
  revision engines, and a shared desktop Library menu.
- [x] Place an image on canvases, open it in the image editor, and propagate edits
  without export/import. Any imported raster can enter this workflow.
- [x] Independent image copies and recursive canvas copies with distinct histories.
- [x] One `spectrum` CLI for library listing, image import/adjustment, canvas
  creation/placement, core JSON commands, copying, and exports; live edits use
  authenticated desktop hosts rather than racing open workspaces.
- [x] Complete CLI help/schema, editing tools, history, collaboration, benchmarks,
  and terminal consolidation under `spectrum images` / `spectrum canvas`.
  Old executable targets and packaged helpers are removed; engines remain internal.
- [x] Backup/restore procedure and rebuildable preview ownership documented.

## Verification

The full fmt/clippy/workspace-test loop passed with `RUST_TEST_THREADS=4`.
Default parallel testing hit one existing cache-worker temporary-file timeout.
Linux packaging and `scripts/spectrum-library-smoke-linux.sh` passed in release:
real editor navigation, live CLI edits/placement, independent copies, and exports.
Integration tests cover two canvases, source-file removal, restart, and undo targeting.
`lumen benchmark --strict` and `prism benchmark --strict --profile hosted-ci` passed.
`prism benchmark --strict` hit four existing gradient workstation budgets on this
host: radial large, angle small/large, and gradient shadow. The unchanged
`68f266e` CI binary fails the same cases (109.5/31.7/141.4/658.4 ms versus
112.5/32.2/144.1/664.3 ms during this run); thresholds were not changed.

The refresh flicker fix passed the full validation loop, release desktop smoke,
and the same strict image/hosted-ci canvas benchmarks. Background polls now stay
silent and leave controls enabled; user actions queue once behind an active poll.

CLI consolidation passed the full fmt/clippy/workspace-test loop, packaged Linux
schema checks and desktop smoke, and both strict benchmarks (canvas: hosted-ci).
Tests cover full command dispatch, asset typing, export options, terminal target
precedence, and existing collaboration sessions. [CLI usage](../docs/CLI.md).

## Next

- Make rough UI mockups with the user, then rebuild in GPUI. Existing layouts
  are not constraints; revisit the canvas horizontal toolbar, library access,
  editor navigation, context preservation, and return paths. Avoid egui redesign.
- Finish library removal/restore and relocation semantics and retire catalog/file
  management affordances as their replacement workflows are established.
- Stable typed animation property addressing remains future work; retain all
  editable properties and structured canvas content as required by direction.
