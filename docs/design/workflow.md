# Projects, editing scope, and the new interface

This records the user's preferences from the design discussion. It is not an
implementation specification. Read [direction](../DIRECTION.md) for the wider
product requirements and [Suite](../SUITE.md) for what currently exists.

## Projects and assets

The intended starting workflow is to create or open a project, then import
assets of any supported kind. Avoid separate photo-shoot and video-import
workflows. A project can contain imported media, canvases, and future video or
music work made inside Spectrum. Photos can still be viewed in useful batches.

Assets live in one Spectrum-wide library. On import, users should be able to
attach the imported assets to an existing or newly created project immediately.
This is the expected common workflow. Reusing an asset in another project must
not require copying it. Projects organize references to shared library assets.
The exact membership controls and project/asset-manager views remain open.
Projects live inside Spectrum and do not require user-managed project files.

The user accepts that a composition is itself an asset that another composition
can use. A canvas could be a still composition, with future audio and video
compositions. The exact taxonomy and compatibility rules remain open. The user
would welcome a shorter name but accepts composition for now. Direct image
editing must fit without requiring a composition for every operation.

### Membership, removal, and import batches (2026-09-27)

The user does not want to assign assets to a project at import. They want to
bring in a shoot first and organize later. Assets may belong to no project; the
library shows them in an unassigned view, and an all-assets view shows everything.

Right-clicking an asset (two-finger click on Mac) opens a menu. In a project it
offers "Remove from project", with helper text saying the asset stays in the
library. "Delete asset" appears in red with a warning that it deletes the asset
from Spectrum. Removing an asset from its last project is fine; it becomes
unassigned. The demo also offers "Add to project" in this menu.

Record which assets were imported together, even before the UI shows it, so a
later view can answer "what did I bring in last week?". Import supports images
only until video and audio work begins. Label imported images "Image", not "Photo".

Deleting asks "are you sure?" and needs a second click. If the asset is in any
projects, the confirmation shows a scrollable list of them; clicking one opens
that project. Deleted assets go to a Spectrum trash for 30 days, then are purged.
Canvases that use a deleted image show a missing-image placeholder, so the user
can restore or replace the image later. Replacing a placeholder with another
image is not built yet.

### Library sidebar and views (2026-09-27)

The Library sidebar order is Import, asset search, then the view button. The view
button keeps its field look but opens a search palette instead of a dropdown, so
many projects stay easy to find. The palette sits horizontally centered in the
content area, not the whole window, and near the top. Sorting and the Images and
Canvases toggles sit below. The separate View and Filter headings are gone. The
user also suggested an import button in the projects view; its placement is open.

### Home, projects, and sidebar modes (agreed 2026-09-27)

The app opens to Home, outside any project. Home has two sidebar modes: Projects
(a searchable project grid with New project) and Assets (the whole library: All
assets, Unassigned, Trash, with Import assets). Opening a project enters its
workspace, where everything is scoped to that project.

Inside a project the main area shows one thing: the project overview grid, an
image, or a canvas (later a video). The sidebar shows exactly one mode at a time,
never assets and controls stacked together; the user dislikes the After Effects
pattern of cramming panels vertically. Modes are capabilities, not asset types:
Assets (the project's assets, used to open items), Color (color correction for
whatever is selected: an image, a canvas layer, later a clip), Layers, and later
Effects, Timeline, and Audio. Only modes that apply to the current selection
appear. A compact fixed strip at the top of the sidebar shows the project and
the mode icons. Opening an image selects Color as a default, but color tools
must not be scoped to images.

Command+1 to 9 switch sidebar modes. Command+K opens the search palette for
projects, places, and assets. Do not bind Command+S: people press it by habit to
save, and Spectrum saves automatically, so it should do nothing.

Follow-up review: New project appears only in the Projects tab and Import assets
only in the Assets tab, styled as the original segmented control. Inside a
project the Home tabs disappear, and back goes up one level at a time.

Third review: "Add to project" opens a search modal like Command+K, listing
projects and offering a new project, so a fresh import can go straight into a
new project. Inside a project, Import assets also brings in assets already in
the library. The back row matches the mode tabs' height so the divider does not
shift. Drag-box selection scrolls when the pointer nears the grid's edge.

Fourth review, general rules the user set:
- Responsiveness: changes must show instantly. Deletes remove cards at once;
  imports show named placeholder cards, and thumbnails fade in only when they
  have just rendered, avoiding loading flicker.
- Toasts are for information the user cannot see, with a way to act on it. Adding
  assets to another project shows a toast that opens that project; adding to
  the project on screen shows none.
- Click selects, double-click opens, everywhere: assets and projects alike.
  Projects support Shift and Command selection and bulk deletion.
- Round every asset preview, so hover and selection rings follow its corners.
- Grid thumbnail size uses − and + buttons around a percentage, not a slider:
  a slider in the title row fought the window's title-bar drag on macOS.

Asset grids center their block, support click, Shift-click ranges, Command-click
toggles, drag-box selection, and Command+A, and apply right-click actions to the
whole selection. Assets can be renamed.

## Shared edits and placement edits

The user agrees that editing the shared asset changes all references, while
editing a placement changes only that use. Shared updates should continue to
reach placements that also have local treatments. Independent copies remain
available when the user wants to break the shared connection.

When opening color correction from an image placed in a canvas, the user wants
an explicit way to choose shared-asset editing or editing only that placement.
The user prefers "Edit locally" and "Edit globally" over "Edit this placement",
which felt confusing. These labels fit the intended developer-oriented audience.
Canvas edits default to local. The user expects to place an asset, change it
for that canvas, and reuse the shared asset elsewhere without those local edits.
Global editing needs a separate deliberate entry point; its location and control
remain open. Do not infer approval for a modal prompt on every edit.

Before a global edit, clearly show what it will affect. Include dependent uses
and compositions, not only project membership, including indirect dependencies.
The interface must distinguish the asset being edited from the work affected
by that edit and keep the current scope understandable.

Ordered, repeatable effects remain required, including color correction, another
effect, and another color correction. Asset effects, placement effects, layers,
and composition effects still need precise ordering and ownership rules.

## Sidebar and content area

Preserve screen height for content. The user prefers vertical browser tabs and
objects to the canvas's multiple horizontal bars. The desired design avoids
horizontal top toolbars across editing views. Use a configurable sidebar with
switchable modes, inspired by the user's experience with VS Code's sidebar.
Allow the user to put the sidebar on either the left or right.

The remaining area should show the work, such as a canvas, the asset being color
corrected, or before/after comparisons. Sidebar modes could include asset
management, composition controls, and color correction. Exact modes and the
relationship between sidebar switching and the main view remain open.

A possible fixed area at the top of the sidebar could show the current tool
and actions across sidebar modes, with the area below switching content. This
is a proposal, not an approved layout. Keep the fixed area compact so it does
not consume much sidebar height. The user is still unsure what should change
with each mode.

Keyboard access matters. The user first suggested a switcher chord such as Command+S
followed by a number; they later ruled out Command+S (see Home, projects, and
sidebar modes). Their examples were Command+S then 2 to switch from asset
management to composition controls, and Command+S then 3 for color correction.
They also suggested direct Command+1 and Command+2 shortcuts. Preserve these
examples through context compaction. They are not approved bindings; conflicts
and discoverability need review.

## Design and implementation sequence

The user dislikes both existing editor layouts and their inconsistent appearance.
The GPUI work should include a deliberate visual redesign. The initial merge's
instruction to preserve the old UI does not constrain that design phase.

Before rebuilding whole editors, make a dummy component page with sliders,
buttons, labels, and other proposed controls. Review and revise it with the user
to agree on appearance and interaction, then reuse the approved components.
Also review rough navigation/layout mockups. The user chose a separate GPUI demo;
the user approved the revised components and visual style on 2026-09-27. See [handoff](handoff.md).

Agree on behavior, then prepare and test the engine and CLI before connecting
new production UI to those operations. Build GPUI incrementally. Add video,
audio editing, and music after the current image/canvas workflow is established.

## Discussion and next approval

Keep the user's examples and uncertainty in these notes through context
compaction. Do not replace them with only a summary of settled decisions.
The user wants concise replies and one next action at a time, with the assistant
tracking progress against the plan. After questioning the narrow local/global
mockup, they chose GPUI and a page of controls to style together as the first
implementation step. New project creation and asset import should follow in
GPUI, using the framework intended for the finished app. Do not start a browser
prototype or a full workspace before that controls page. The separate demo now coexists with the current app. Preserve the existing
working editors during the transition. The controls page is not approval to replace those editors.

## First demo review

After the first GPUI demo, the user said GPUI was the right call. They liked the
stock sliders, switches, buttons, and icons, which looked far more modern than
the egui app. Their requested changes:

- The theme sat between dark and middle gray. Move closer to black, a dark
  gray-black rather than pure black.
- The macOS close, minimize, and zoom buttons felt cramped, and the "Spectrum"
  heading under them looked odd.
- A GPT model wrote the first demo, and it showed. Remove filler copy and
  decoration aggressively.
- The sidebar is not only navigation. It holds most controls, and the main area
  shows assets, the canvas, or future video tracks and preview.
- Dropdowns are acceptable, but items need a small gap. Hovering the item above
  the selected one made the highlights touch.
- The stock color picker will not work. Spectrum needs a full color selector;
  the user is unsure whether to build it now.

## Browser deployment research

The user subsequently requested a performance comparison before the GPUI work
and introduced a browser UI with a local native engine as a possible direction.
See [UI framework research](ui-framework-research.md) for findings and limitations.
The user subsequently chose GPUI. Browser deployment is optional and must not
constrain the native implementation. Start a separate GPUI controls demo, then
project creation and asset import after visual review. CI packages the new demo
instead of the old desktop app; retain the old app's source and shared engines.

The supplied ChatGPT desktop screenshot establishes a neutral dark gray direction.
Use spacious layouts, soft rounding, restrained borders, and simple controls.
Avoid strong accent colors and decorative personality so varied creative work
looks at home. The screenshot is a style reference, not a request to copy its
horizontal bars or chat layout.
