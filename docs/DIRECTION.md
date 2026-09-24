# Spectrum direction

Spectrum is intended to become one native Rust creative app. It should bring
photo development, layered composition, video editing, audio editing, and music
making into one workspace while keeping the editing views focused on their
respective work. The current Lumen photo/catalog and Prism canvas applications
are the starting point, not permanent separate products. A user should be able
to find their catalogs and canvases in Spectrum and use work from one editor in
another without repeated export and import steps. The exact navigation, asset
links, and project format remain to be designed.

The near-term sequence is to combine Lumen and Prism into a thin Spectrum app
using the current UI, explore how their workspaces and assets connect, then
rebuild the established interface in GPUI. Avoid a full redesign during the
initial combination. Keep shared behavior in app-neutral crates and expose
persistent edits through core commands and CLIs. [Suite architecture](SUITE.md)
describes the current implementation. **Bloom** was previously reserved for a
separate video and motion editor; the unified Spectrum direction supersedes
that separate-app plan.

## Creative history and collaboration

Completed semantic actions persist automatically. A drag, brushstroke, or
committed text edit should become one revision; transient pointer samples should
not. Human and agent sessions have separate cursors in an immutable revision
tree. Navigating history does not rewrite it, and editing from an older node
creates another future. Do not require users to manage branches manually or
silently discard later work. [The shared revision crate](../crates/spectrum-revisions/src/lib.rs)
owns storage; app adapters own document meaning.

History should be easy to inspect as a visual timeline with previews,
checkpoints, and clear actor attribution. Keep all history by default. Any
future pruning needs an explicit, reviewable user decision. A portable project
should carry its authoritative history and assets in one visible file, while
private caches remain rebuildable. New versions should preserve older states
and let older compatible releases open the history they understand.

Live collaboration is vendor-neutral and CLI-first. Agents should inspect an
open app, submit attributed semantic commands to its authenticated host, and
receive coherent results without silently overwriting human edits. Together
and separate sessions support different creative workflows. The remaining UX
and retention work is tracked in [revision lifecycle](../tasks/revision-lifecycle.md).

## Current product work

Prism remains in progress. The [toolbar overflow prototype](../tasks/toolbar-overflow.md)
was closed without selecting a design because a broader interface overhaul is
planned. Other unfinished work is in the [task directory](../tasks/README.md).
