# Spectrum direction

Spectrum is a suite of focused, fast, native Rust creative tools. Lumen owns
photo development and catalogs; Prism owns layered composition. **Bloom** is
the confirmed name for the future single video and motion editor. Its name is
a product decision, not an implementation commitment. Keep shared behavior in
app-neutral crates and expose persistent edits through core commands and CLIs.
See [suite architecture](SUITE.md).

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
was closed without selecting a design because a broader Prism overhaul is
planned. Other unfinished work is in the [task directory](../tasks/README.md).
Keep Lumen's photo workflow and Prism's canvas workflow focused as they mature.
