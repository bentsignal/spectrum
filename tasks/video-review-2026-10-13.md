---
status: in_progress
priority: urgent
---

# Video editor review, 2026-10-13

The user's first pass on the editor (preview `afdfb80`, their camera MP4s at
about 1 GB a minute). They liked how it feels. The main complaint: the Edit
tab became a pile of buttons, which breaks the focused-tabs rule. Tools
belong in a button and a Command+P list (as on canvases), and actions belong
in keys, commands, and right-click menus. Speed modes also lost work. Part of
[video and animation](video-and-animation.md).

All twenty items were built on 2026-10-13 (PR #113). In the second pass
(2026-10-15), the user found the timeline grip dropping drags and capped
too low. Drags now follow the pointer window-wide, and the grip goes up to
a sliver of monitor; a double-click resets it. The tracks are one list that
scrolls as one, Vegas-style: drag a track's name to put it anywhere, and
higher video tracks are in front. The project sidebar has Import and New
dropdowns and a quiet list of kinds: a click shows only that kind, and
Command-click or a right-click adds kinds.

A third pass (2026-10-15) settled tracks in any order. It also brought:
- The transport's top edge resizes too; Escape refocuses after Command+P.
- Keyframe values are typed from a keyframe's right-click menu, with a value
  tooltip while dragging. The Speed tab dropped its keyframe list and curve
  row.
- Track moves show live. "Add media…" is in Command+P (arrows and Enter).
- Command+P rows show what each item does.
- There is a Video tab (size, rate, background) and a fuller Export tab.
- Retired images wait for time as well as renders, after the atlas crash.

Left:
- A tall time remap graph, which comes with Animate.
- Preview frames don't see edits made to an image or canvas the video uses.
