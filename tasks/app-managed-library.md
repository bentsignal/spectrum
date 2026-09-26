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

The initial integration and refresh fix passed workspace validation, release
Linux desktop smoke, and strict image and hosted-ci canvas benchmarks. Four
workstation gradient budgets also failed on the unchanged `68f266e` baseline;
thresholds were not relaxed. Git retains the detailed measurements and fixes.

CLI consolidation passed the full fmt/clippy/workspace-test loop, packaged Linux
schema checks and desktop smoke, and both strict benchmarks (canvas: hosted-ci).
Tests cover dispatch, targeting, export, and collaboration. See [CLI usage](../docs/CLI.md).

## Next

- [Workflow design](../docs/design/workflow.md) records the user's project, asset,
  shared/local editing, sidebar, keyboard, and component-page preferences.
  Settle membership and effect ordering; review layout and component mockups.
  Prepare engine/CLI behavior, then build the agreed GPUI interface gradually.
  Documentation checks passed; these preferences are not implemented yet.
- Finish library removal/restore and relocation semantics and retire catalog/file
  management affordances as their replacement workflows are established.
- Stable typed animation property addressing remains future work; retain all
  editable properties and structured canvas content as required by direction.
