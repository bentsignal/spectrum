# Spectrum direction

Spectrum is one native Rust creative app. It should bring photo editing,
layered composition, video editing, audio editing, and music making into one
workspace while keeping each editor focused on its work. Spectrum began as two
apps, Lumen (photos) and Prism (canvases); that split is gone. A user should
find all their photos and creative work in Spectrum and use work from one
editor in another without export and import steps.

## App-managed creative library

Spectrum is greenfield. It has not shipped, and there is no backward
compatibility: it reads one storage format, and a library may be wiped when that
format changes. Do not add migrations or version ladders for unreleased formats.

Open Spectrum and start working. Imported photos, canvases, and future video and
audio work live in one app-managed library; users never choose file names or
locations. Each asset is one document in the library, and the library index is
authoritative for identity, names, projects, and references. Export and sharing
are deliberate actions that write outside the library. Library-level backup and
restore UI remain to be designed; copying the library directory works today.

Spectrum has one CLI; video, audio, and music commands will follow images and
canvases. The CLI and the GUI must both expose every creative operation. Add
commands before or alongside GUI controls, and keep validation, persistence,
history, and asset behavior below both. GUI presentation state can stay in the
interface.

### Organization across media

Projects group library assets without owning them; an asset can be in several
projects or none. Import may target a project, but need not. Unassigned assets
and an all-assets view keep loose imports usable. Removing an asset from a project
keeps it in the library. Deleting asks for confirmation, lists the asset's
projects, and moves it to a trash that purges after 30 days. Canvases that use a
deleted image draw a same-sized placeholder so it can be restored or replaced. Each
import is recorded as a batch for later "imported together" views. Images are the
only importable kind today. See [workflow design](design/workflow.md).

### Images, canvases, and video

Decided 2026-10-05: an image is its own asset, not a canvas with one photo
layer. The image editor does color correction, crop, and geometry. For more,
place the image on a canvas, which composes images with text, shapes, paint,
masks, and effects. Video will compose images, canvases, audio, and video.

Every asset has a type that decides its editors and uses. Types cover images,
canvases, audio, video, and music compositions from a future full DAW; the music
type's final name is open. Canvases use images; video uses everything. Audio
does not belong in the image or canvas editors.

References follow asset edits by default, transitively: editing an image updates
every canvas that uses it, and every video using those canvases. Explicit deep
copy creates independent content whose changes do not propagate. Local versus
global editing of an image placed on a canvas needs a deliberate scope choice
("Edit locally", "Edit globally"); defaults and controls remain open.

### Ordered effects

Canvases and video need repeated color correction interleaved with other
effects: for example color correction, then an effect, then a smaller color
correction. Order must affect the result; do not hard-code one color stage
followed by one effects stage. The user sometimes edits stills in After Effects
because Lightroom cannot express layered treatments; Spectrum should support
that through canvases without making still work depend on video.

### Video and animation

Decided 2026-10-13, designed as one piece ([plan](../tasks/video-and-animation.md)):

- **Footage** (imported video with its sound) and **audio** are imported
  assets, copied into the library so the card can be wiped. Spectrum is for
  public release: read and export the major codecs and containers (FFmpeg).
- A **video** is an edit: Premiere-style tracks of clips that use footage,
  audio, images, canvases, and other videos.
- **No animation asset.** Every editable property, image adjustments and
  toggles included, is a value or keyframes (switches hold). A canvas with
  keyframes is animated and plays in a video. Edit (tracks) and Animate
  (property keyframes and graph) are two views of one timeline: animate a
  clip or a canvas's layers, go back, and see it.
- **Speed** per clip is either velocity keyframes (Sony Vegas) or a time
  remap (After Effects), not both. Neither changes the clip's length on the
  timeline: running out of footage repeats it, as Vegas does. Reverse and
  freeze are required. Slowing below the project's frame rate offers
  easy, high-quality frame interpolation. Audio follows the speed by
  default, with a one-click switch to leave it unchanged.

### Interface

The desktop app is native GPUI. The user wants a switchable left or right
sidebar holding the current mode's controls, content using the full window
height, and no horizontal toolbars. [Workflow design](design/workflow.md)
records preferences, review history, and unsettled choices. Interaction latency
is a product priority: measure regressions with the strict benchmarks rather
than polishing speculatively.

## Creative history and collaboration

Completed semantic actions persist automatically. A drag, brushstroke, or
committed text edit becomes one revision; transient pointer samples do not.
People and agents have separate sessions with their own cursors in an
immutable revision tree. Navigating history never rewrites it, and editing from
an older revision keeps the later work as another branch. Users should not
manage branches manually or lose later work silently.

History should be easy to inspect as a visual tree or timeline with previews,
checkpoints, and clear attribution of who did what. Keep all history by default;
any pruning needs an explicit, reviewable user decision. History and embedded
files belong to the library, with rebuildable private caches.

Collaboration is vendor-neutral and CLI-first: every change a person can make
in the app, an agent can make through the CLI. An agent starts from where the
person is and works in its own session; its work never overwrites the
person's, because revisions are immutable and the person's place moves only by
their own choice. Working together, the person follows the agent's revisions
and watches them land until they edit, which branches. Working separately,
the person never moves. The person can jump to any revision, an agent's
included, and editing there branches from it. The History mode shows each
image's and canvas's tree and where each session is; clicking a revision or
branch goes there. People name branches, not single revisions, and find
them by name in the palette; the person can watch a session live and go back to their own work. Remaining work is in
[history tree](../tasks/history-tree.md) and
[revision lifecycle](../tasks/revision-lifecycle.md).

## Current product work

Unfinished work is in the [task directory](../tasks/README.md); start with the
[overhaul follow-ups](../tasks/overhaul-follow-ups.md).
