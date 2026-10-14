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

All twenty items were built on 2026-10-13 and await the user's review:

- Imports clone files on the same volume, and the cards show progress.
- A swipe up or down zooms; sideways scrolls time; Option or the track names
  scroll tracks.
- Video and audio panes scroll and grow from the divider. A grip sets the
  timeline's height, and the divider sets the split.
- There is a tool button and Command+P list, plus right-click menus. Shift+E
  enables or disables a clip, Command+L links or unlinks, and I, O, Option+X
  and Enter mark and render.
- The tabs are Clip, Speed, Media (a searchable grid), Export, and History.
- Velocity and time remap are two views of one timing. The velocity scale is
  non-linear, a double-click adds a keyframe and drags it, and a right-click
  sets its curve.
- The razor shows a cut line. Stopping returns to where play began (a
  switch, on by default). Preview renders show green on the ruler.
- The open video's footage gets its playback copy first, and the monitor
  says when one is still being made.

Left:
- A tall time remap graph, which comes with Animate.
- Preview frames don't see edits made to an image or canvas the video uses.
  The video's own stills share that gap.
