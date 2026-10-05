# Spectrum direction

Spectrum is intended to become one native Rust creative app. It should bring
photo development, layered composition, video editing, audio editing, and music
making into one workspace while keeping the editing views focused on their
respective work. The current Lumen photo/catalog and Prism canvas applications
are the starting point, not permanent separate products. A user should be able
to find all their photos and creative work in Spectrum and use work from one
editor in another without repeated export and import steps. The exact
navigation remains to be designed; asset behavior is defined below.

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

Spectrum has one CLI with images and canvas commands; video, audio, and music
commands will follow. Both the CLI and GUI must expose all creative operations. Add commands before or alongside
GUI controls; keep validation, persistence, history, and asset behavior shared.
Full existing-feature parity still needs an audit. GUI presentation state can
remain specific to the interface.

### Organization across media

Projects group library assets without owning them; an asset can be in several
projects or none. Import may target a project, but need not. Unassigned assets
and an all-assets view keep loose imports usable. Removing an asset from a project
keeps it in the library. Deleting asks for confirmation, lists the asset's
projects, and moves it to a trash that purges after 30 days. Canvases that use a
deleted image draw a same-sized placeholder so it can be restored or replaced. Each
import is recorded as a batch for later "imported together" views. Images are the
only importable kind today. See [workflow design](design/workflow.md).

### Color correction and ordered effects

The current Photos view mainly provides color correction, plus crop and
transforms. Treat color correction as a capability for compatible visual assets,
including images used in canvases and future video. A permanent top-level
Photos/Canvas split is not decided. A dedicated color workspace is a proposal;
its name, placement, and relationship to other editing tools remain open.

Support repeated color correction operations interleaved with other effects.
A required example applies color correction, then an effect, then another color
correction for a smaller change. Processing order must affect the result. Do not
hard-code a single color stage followed by a separate effects stage. Layers and
effects must support this editing approach, including future animation of their
editable properties. Compatible operations should be available where the asset
is being edited, without export/import detours or artificial editor boundaries.

The user sometimes edits still photos in After Effects because Lightroom cannot
express the desired layered treatments. Spectrum should support that workflow
without making still-image work depend on a video workflow. Define which edits
belong to the shared asset, a particular use of it, a layer, or a composition.
The user accepts shared-asset edits versus placement-only edits, with an explicit
scope choice when editing from a placement. Defaults and controls remain open,
as do adjustment layers, masks, and group/composition effects.

### Typed assets and connected editors

Every library asset has a type that determines compatible editors and uses.
Types must accommodate images, canvases, audio, video, and future editable music
compositions from a full DAW; the music type's name remains undecided. Any image,
including graphics imported while composing a canvas, can use the image editor.
Images can be referenced by canvases; canvases and audio can be used in video.
Audio does not belong in the image or static canvas editor.

References follow asset edits by default, transitively: editing an image updates
every referencing canvas and every video using those canvases. Moving between
editors must preserve those connections without export/import. Explicit deep
copy creates independently editable content whose changes do not propagate to
or from the original. Current copy and history behavior is in [Suite](SUITE.md).

Future animation should address every editable creative property, including
image adjustments and toggles, while retaining editable canvas structure.
Continuous values and discrete switches need appropriate animation semantics.
Keep this possible in the model; implementing animation or the DAW is later work.

Desktop/CLI consolidation and initial asset links are complete. Next, agree on
behavior and review layout mockups and a dummy component page before a gradual
GPUI redesign. Prepare engine/CLI behavior before connecting production UI.
The user wants a switchable left/right sidebar and content using the screen
height, without horizontal top toolbars. Existing layouts are not constraints.
[Workflow design](design/workflow.md) records preferences and unsettled choices.
Video, audio, and music follow the current workflow; Bloom is not a separate app.

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

The codebase is being reshaped around one app ([overhaul](../tasks/codebase-overhaul.md)).
Other unfinished work is in the [task directory](../tasks/README.md).
