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

## Shared edits and placement edits

The user agrees that editing the shared asset changes all references, while
editing a placement changes only that use. Shared updates should continue to
reach placements that also have local treatments. Independent copies remain
available when the user wants to break the shared connection.

When opening color correction from an image placed in a canvas, the user wants
an explicit way to choose shared-asset editing or editing only that placement.
The user prefers "Edit locally" and "Edit globally" over "Edit this placement",
which felt confusing. These labels fit the intended developer-oriented audience.
The default scope and choice of prompt, separate actions, or another control
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

Keyboard access matters. The user suggested a switcher chord such as Command+S
followed by a number. Their examples were Command+S then 2 to switch from asset
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
Also review rough navigation/layout mockups. Whether to prototype in the current
framework or elsewhere is unresolved; no egui redesign has been approved.

Agree on behavior, then prepare and test the engine and CLI before connecting
new production UI to those operations. Build GPUI incrementally. Add video,
audio editing, and music after the current image/canvas workflow is established.

## Discussion and next approval

Keep the user's examples and uncertainty in these notes through context
compaction. Do not replace them with only a summary of settled decisions.
The user wants concise replies and one next action at a time, with the assistant
tracking progress against the plan. Proposed next action: make one rough
clickable sidebar/workflow mockup using an image shared by two canvases. Await
the user's thumbs-up before starting that prototype. This documentation update
does not authorize implementation or a production UI rewrite.
