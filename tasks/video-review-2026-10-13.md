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

1. Importing large MP4s is slow: find where the time goes.
2. A two-finger swipe up or down zooms the timeline (the wheel in Vegas). A
   sideways swipe or Shift-wheel scrolls time.
3. Video and audio tracks scroll separately, as in Premiere. Scroll over the
   headers, or hold Option, to scroll them.
4. Drag a video track's top edge or an audio track's bottom edge to resize
   it; tracks grow away from the video/audio divider.
5. A draggable divider sets how tall the monitor and the tracks are; it is
   kept.
6. Keyframe dots stop drawing over the track headers.
7. Tools: a tool button and a Command+P list. Letter keys stay.
8. Clip actions are keys, Command+P commands, and a right-click menu.
9. Tabs: Clip, Speed, Media, Export, History.
10. After Unlink, a click on one clip of a selection selects just that clip.
11. Media tab: a grid of thumbnails with search; drag onto the timeline.
12. Velocity scale: 0% to 300% takes most of the height, with the extremes
    squeezed toward the edges. Guide lines at 0% and 100%; values typeable.
13. Double-clicking a speed line adds a keyframe that drags at once.
14. Right-click a keyframe to choose its curve (Vegas's six).
15. Velocity and time remap are two views of one timing. Switching views
    loses nothing; editing in the other view converts the keyframes,
    accurate to the frame, in one undo step. No Normal mode; Reset speed is
    a command.
16. A tall time remap graph comes with Animate; until then rows grow.
17. The razor shows where the cut falls and a razor at the pointer.
18. A "return to start when playback stops" switch, on by default.
19. Preview render: render In to Out (I and O) or the whole edit ahead so it
    plays perfectly. The ruler shows what is rendered, and an edit clears
    only the frames it changes.
20. Slow first frames after play or a jump: footage in the open video gets
    its playback copy first, and the monitor says while one is being made.
