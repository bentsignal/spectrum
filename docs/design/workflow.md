# Projects, editing scope, and the new interface

This records the user's preferences from the design discussion. It is not an
implementation specification. Read [direction](../DIRECTION.md) for the wider
product requirements and [Suite](../SUITE.md) for what currently exists.

## Projects and assets

The intended starting workflow is to create or open a project, then import
assets of any supported kind. Avoid separate photo-shoot and video-import
workflows. A project can contain imported media, canvases, and future video or
music work made inside Spectrum. Photos can still be viewed in useful batches.

Projects organize work, but must allow easy reuse of assets across projects.
The user wants asset management and access to all projects. Whether these need
separate views, how membership works, and how references cross project boundaries
remain design questions. Projects live inside the managed library; this does
not reintroduce user-managed project files.

A possible distinction is imported media versus editable compositions made from
other assets. A canvas could be a still composition, with future audio and video
compositions. The user is exploring this terminology and has not approved a
fixed taxonomy or compatibility rules based on those names. Direct image editing
must also fit without requiring a composition for every operation.

## Shared edits and placement edits

The user agrees that editing the shared asset changes all references, while
editing a placement changes only that use. Shared updates should continue to
reach placements that also have local treatments. Independent copies remain
available when the user wants to break the shared connection.

When opening color correction from an image placed in a canvas, the user wants
an explicit way to choose shared-asset editing or editing only that placement.
The labels, default scope, and whether this is a prompt, separate actions, or
another control remain open. Do not infer approval for a modal prompt on every
edit. The interface must make the current edit scope understandable.

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

Keyboard access matters. The user suggested a switcher chord such as Command+S
followed by a number, or direct Command+number shortcuts. These are examples,
not approved bindings; conflicts and discoverability need review.

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
