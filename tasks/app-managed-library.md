---
status: todo
priority: high
---

# Design Spectrum's app-managed creative library

Spectrum is greenfield: old names and formats require no backward compatibility.
Users should import images, create work, and find it without managing project
files. The library serves image, canvas, and future video, audio, and music
workspaces. See [product direction](../docs/DIRECTION.md).

Decide how Spectrum identifies and stores assets and work, handles originals,
history, backup, portability, recovery, and explicit export or sharing. Design
the one `spectrum` CLI with domain commands such as `images` and `canvas` to
address the same library. Do not add file-format migration requirements by
default or preserve separate Lumen/Prism public product names.

Start with one image referenced by two canvases, editable in the image workspace
without export/import. Both follow edits; an explicit deep copy stays independent.
Expose these operations through the unified CLI alongside the existing views.
Current `from-lumen` renders a PNG snapshot; raster layers hold paths and photo
IDs are catalog-local. Revision `AssetId`s identify immutable bytes. Reuse engines,
commands, and revision storage, adding shared editable identity and references.
Design type compatibility, transitive invalidation, cycle handling, copy scope,
and coherent history before implementation. Allow stable typed property addresses
for future animation; composition-local animation overrides remain a proposal.

## Acceptance criteria

- An agreed storage and asset identity model supports work across editors.
- Images shared across canvases update live; deep copies edit independently.
- Starting, importing, finding, and editing work do not require choosing a
  project-file path.
- Backup, recovery, and user-directed export or sharing are defined.
- One Spectrum CLI can address each implemented domain through the same model.
