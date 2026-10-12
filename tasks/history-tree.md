---
status: in_progress
priority: high
---

# History tree

Every edit to an image or canvas is a revision in an immutable tree, and each
person and agent moves through it in their own session
([durable documents](../docs/ARCHITECTURE.md)). The person needs to see that
tree and work with it.

## What the user asked for (2026-10-11)

- See which revision they are on in the tree of saves, and every path they
  have taken while working.
- Go back to any point and build from there (editing there branches).
- Watch another session's branch as it develops, such as an agent working,
  and branch off from the agent's work.
- Name branches and search for them, so large trees stay navigable.
- A 2D view like a canvas: pan with the mouse and two fingers, zoom in and
  out.
- Fast on every platform, like everything else.

## Plan

Engine and CLI first, then the app on top of the same service calls.

1. **Names.** People name branches, not single revisions: the name goes on
   the revision that began the branch. Naming is not an edit and adds no
   revision. Stored with the history; `history` output includes names. CLI:
   `history-name <revision> <name>` (the branch holding it) and `--clear`,
   for images and canvases.
2. **Search.** `history --search <text>` matches names, edit descriptions,
   and who made them.
3. **Previews.** A small preview of any revision, rendered without moving
   any session, kept in a rebuildable cache outside the history.
   CLI: `history-preview <revision>` prints the cached file.
4. **Watching.** The person can watch any session: the open asset follows
   that session's revisions as they land, until the person edits, which
   branches. CLI: `history-jump --session <id>` moves to a session's place.
5. **The History view** for images and canvases: a sidebar mode with search,
   the branches (name or who, when, who is there, Watch), and the selected
   revision's details (name it, go there); the main area shows the tree,
   time left to right and branches stacked, with previews when zoomed in,
   each session's place marked, and the current revision highlighted. Pan
   and zoom like a canvas. Clicking a revision selects it; going there moves
   the person, and editing then branches.
6. **Checks.** The interaction benchmark opens, pans, and zooms a history of
   thousands of revisions with branches, and jumps; the feature checks name,
   search, jump, and watch.

## Progress

- 2026-10-11: steps 1-6 built. CLI: `history --search`, `history-name`,
  `history-preview`, `history-jump --to-session` (tests in
  `apps/spectrum/tests/history_cli.rs`). App: History mode (⌘4 on images,
  ⌘6 on canvases) with the tree, search, names, going to a revision, and
  watching; the benchmark's `history` scenario covers it on 1,200 revisions.
- 2026-10-11, after the person's first review: clicking a revision or a
  branch goes there; Back and Forward; names are on branches (the revision
  that began each); the sidebar shows the person's revision whole and sharp,
  keeping the last preview until the next is ready; watching starts at once
  (about 200 ms to show an agent's edit).
- Second review: branches are found in the palette (⌘P in History, or "Find
  a branch") instead of a sidebar list and search; typing a branch name is
  never overwritten by a reload; Crop and Color draw in place from the first
  frame (the crop view measures the same area as the image view).
- Third review: the sidebar opens with the branch the person is on, styled
  like the canvas's tool button, which opens the branch finder; branch names
  in the tree go to their branch when clicked. The branch name field
  scrolled its text out of view on the person's Mac; it did not reproduce on
  Linux at 1x, 2x, or with the sidebar scrolled.
- Fourth review: found on a macOS CI runner (screenshots plus a logged copy
  of the input library, branch `debug/mac-input`): on macOS a text field
  sized by its container lays its text out 0 px wide (233 on Linux), so
  focusing scrolled the text out of view. Sidebar fields now get an explicit
  width (`controls::FIELD_WIDTH`), confirmed on the runner. The hint under
  the branch field is gone.
  Next: keyboard navigation, previews when a canvas's linked images change,
  very wide histories.

## Acceptance

- Each step works from the CLI and in the app, saved through
  `spectrum_assets::Service`.
- The full validation loop passes, including the new benchmark scenarios.
