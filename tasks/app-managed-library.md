---
status: todo
priority: high
---

# Design Spectrum's app-managed creative library

Spectrum is a greenfield product. The current Lumen/Prism names and project
files are temporary implementation details; no shipped users or real projects
require backward compatibility. Users should open Spectrum, import photos or
create work, and find all their material in the app without naming or locating
project files. The library must serve photo, canvas, and future video, audio,
and music workspaces. See [product direction](../docs/DIRECTION.md).

Decide how Spectrum identifies and stores assets and work, handles originals,
history, backup, portability, recovery, and explicit export or sharing. Design
the one `spectrum` CLI with domain commands such as `images` and `canvas` to
address the same library. Do not add file-format migration requirements by
default or preserve separate Lumen/Prism public product names.

## Acceptance criteria

- An agreed storage and asset identity model supports work across editors.
- Starting, importing, finding, and editing work do not require choosing a
  project-file path.
- Backup, recovery, and user-directed export or sharing are defined.
- One Spectrum CLI can address each implemented domain through the same model.
