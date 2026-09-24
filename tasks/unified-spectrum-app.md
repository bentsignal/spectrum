---
status: in_progress
priority: high
---

# Bring photo and canvas editing into one Spectrum app

Create one Spectrum desktop app that hosts the existing Lumen photo/catalog and
Prism canvas workspaces. Keep their command engines, project files, and focused
editing views intact for this first combination. Provide clear in-window
navigation between workspaces and route opened documents to the right editor.
The GPUI rebuild and new cross-workspace asset links follow after this shell is
usable; they are not part of the initial merger.

Replace the standalone Lumen and Prism app packages and desktop launchers with
the Spectrum app. Preserve the `lumen` and `prism` CLIs for existing automation
while the unified command surface is designed.

## Acceptance criteria

- One Spectrum window opens both kinds of workspace and switches without
  losing the state of either editor.
- Existing `.lumen` and `.prism` files open in their respective workspace;
  existing editing, history, export, and terminal flows remain available.
- Linux, macOS, and Windows builds and packages expose one Spectrum GUI app.
- The full required validation loop and affected packaging checks pass.
