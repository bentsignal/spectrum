---
status: done
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

## Outcome

The Spectrum desktop app now hosts the existing Lumen and Prism views in one
window. A small workspace switcher moves between Photos and Canvas while each
editor retains its state. Existing document types open in their respective
workspaces. The separate GUI packages and launchers have been replaced by one
Spectrum package; the `lumen` and `prism` CLIs remain available.

The required format, Clippy, and test loop passed locally. The Linux package
was built and its binaries verified locally; the Photos to Canvas to Photos
flow was checked in an isolated display. [CI run 36053949146](https://github.com/bentsignal/spectrum/actions/runs/36053949146)
passed format, lint, tests, and Linux, macOS, and Windows release packaging.
