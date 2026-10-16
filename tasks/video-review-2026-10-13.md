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

Built and awaiting review: the first pass (PR #113); the second (#114:
drags held window-wide, Vegas-style single track list, quieter project
sidebar); the third (#115: focus fixes, add media from Command+P, Video and
Export tabs, live track moves). The fourth (2026-10-15) brought:
- Focus returning to the editor whenever it is lost.
- Backward sound that reads ahead; it seeked every step and froze playback.
- VideoToolbox exports that start sharp.
- Fade handles and Vegas crossfades (cross a neighbor), picture and sound.
- Speed lines on every clip, trim and In/Out cursors, and Shift snapping.
- The canvas color picker for the background, typed rates, and an aspect
  lock.

Left:
- A tall time remap graph, which comes with Animate.
- Preview frames don't see edits made to an image or canvas the video uses.
