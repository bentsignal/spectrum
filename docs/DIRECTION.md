# Spectrum direction

Spectrum is intended to become one native Rust creative app. It should bring
photo development, layered composition, video editing, audio editing, and music
making into one workspace while keeping the editing views focused on their
respective work. The current Lumen photo/catalog and Prism canvas applications
are the starting point, not permanent separate products. A user should be able
to find all their photos and creative work in Spectrum and use work from one
editor in another without repeated export and import steps. The exact
navigation and asset links remain to be designed.

## App-managed creative library

Spectrum is a greenfield product. It has not shipped to users, and the current
work has not been used for projects that require preserving old formats or
workflows. Backward compatibility with `.lumen`, `.prism`, `.lumencatalog`, or
`.mica` is not a product requirement. Those files and the Lumen and Prism names
describe the current implementation, not the destination. Do not let migration
support dictate the new architecture.

The target experience is to open Spectrum and start working. Imported photos,
canvases, and future video and audio work should be discoverable from one
app-managed creative library without choosing project-file names and locations.
Spectrum may organize its internal data as needed; users should not have to
manage that layout. The app should provide deliberate ways to export or share
work when needed. The library's storage, identity, backup, portability, and
export model remain design questions. Avoid assuming that a traditional
project-file workflow is the answer.

The public command surface should likewise become one `spectrum` CLI with
domains such as `images`, `canvas`, `video`, `audio`, and `music`. The existing
Lumen and Prism command engines can inform this transition, but their product
names and separate executables are temporary.

The near-term sequence is to combine Lumen and Prism into a thin Spectrum app
using the current UI, explore how their workspaces and assets connect, then
rebuild the established interface in GPUI. Avoid a full redesign during the
initial combination. Keep shared behavior in app-neutral crates and expose
persistent edits through core commands and CLIs. [Suite architecture](SUITE.md)
describes the current implementation. **Bloom** was previously reserved for a
separate video and motion editor; the unified Spectrum direction supersedes
that separate-app plan.

Interaction latency is a product priority. Before the GPUI rebuild, keep
performance work focused on measured regressions and costs that will survive
the rewrite. Avoid a broad optimization or polish pass on disposable egui UI
code. Changing UI libraries alone does not establish that image development,
storage, or preview scheduling will meet the intended responsiveness.

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
future pruning needs an explicit, reviewable user decision. Authoritative
history and assets should belong to Spectrum's managed library, with rebuildable
private caches. Users should not have to track visible project files.

Live collaboration is vendor-neutral and CLI-first. Agents should inspect an
open app, submit attributed semantic commands to its authenticated host, and
receive coherent results without silently overwriting human edits. Together
and separate sessions support different creative workflows. The remaining UX
and retention work is tracked in [revision lifecycle](../tasks/revision-lifecycle.md).

## Current product work

Prism remains in progress. The [toolbar overflow prototype](../tasks/toolbar-overflow.md)
was closed without selecting a design because a broader interface overhaul is
planned. Other unfinished work is in the [task directory](../tasks/README.md).
