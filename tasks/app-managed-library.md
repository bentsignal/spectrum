---
status: in_progress
priority: high
---

# Complete Spectrum's app-managed creative library

See [direction](../docs/DIRECTION.md) and [library architecture](../docs/SUITE.md).
The user verified linked editing and the CLI on Mac; validation history is in Git.

## Implemented

Typed library assets, dependencies, managed documents, shared image/canvas edits,
independent copies, and the consolidated CLI are complete. Authenticated live
editing, backup/restore documentation, and preview ownership are established.
See Git for implementation and validation history.

## Next

- [Workflow design](../docs/design/workflow.md) records the user's project, asset,
  shared/local editing, sidebar, keyboard, and component-page preferences.
  The user chose GPUI. See the [demo revision](gpui-demo-design-pass.md); project/import
  workflows follow visual approval. Browser use is optional.
  Prepare engine/CLI behavior, then build the agreed GPUI interface gradually.
- Finish library removal/restore and relocation semantics and retire catalog/file
  management affordances as their replacement workflows are established.
- Stable typed animation property addressing remains future work; retain all
  editable properties and structured canvas content as required by direction.
