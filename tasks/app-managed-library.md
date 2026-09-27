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

- Projects, unassigned assets, and import batches exist in the index and CLI.
  Next: permanent asset deletion (open questions in [workflow design](../docs/design/workflow.md)),
  then GPUI project and import views on the shared engine. Browser use is optional.
- Finish library removal/restore and relocation semantics and retire catalog/file
  management affordances as their replacement workflows are established.
- Stable typed animation property addressing remains future work; retain all
  editable properties and structured canvas content as required by direction.
