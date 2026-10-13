---
status: todo
priority: high
---

# Video and animation editors

The user's intent (2026-10-13): Spectrum takes motion next, designed as one
piece because the two editors intertwine.

- **Video editor**, Premiere-style: video and audio tracks, cutting, trimming,
  the usual editing tasks.
- **Time remapping** like After Effects and **velocity keyframes** like Sony
  Vegas. Vegas is in the user's workflow only for velocity; matching it
  removes an app.
- **Animation editor**, After Effects-style keyframing of layers and effects,
  without After Effects' weakness as an editor (a video becomes 100 layers
  that would be a few tracks in Premiere). Hop between editing and animating:
  animate one clip (a zoom, any property), go back to the edit, see it.
- **Animate almost any property of a canvas**: make a canvas feel alive, go
  back to the canvas to add text, return to animating.
- Effects spread across the app so a still never needs a motion tool for one
  effect (see [Direction](../docs/DIRECTION.md), "Ordered effects").
- New asset types for video and audio; the user doubts animation needs one.

## Proposed model (awaiting the user's answers)

- Assets: imported video footage and imported audio are new kinds, immutable
  like images. A **video** is the edit: tracks of clips that use footage,
  audio, images, canvases, and other videos.
- No animation asset. Keyframes are part of every document: any property is a
  constant or keyframes (continuous values interpolate with easing; switches
  hold). A canvas is animated once it has keyframes and a duration, and plays
  on a video track. Clip keyframes live in the video.
- Two views of one timeline and playhead: **Edit** (tracks and clips) and
  **Animate** (the selection's properties as keyframe lanes with a graph).
  Animate works on a clip in a video or the layers of a canvas. Grouping
  clips into an animated canvas gives After Effects-style layer work that
  appears as one clip in the edit.
- Speed: one curve per clip from timeline time to source time, edited as a
  velocity graph (Vegas) or a time-remap graph (After Effects).
- Engine and CLI first: a neutral time and keyframe crate, a media crate
  (decode, encode, frame cache, audio output), then commands and GUI.
  Playback at speed likely needs the [GPU compositor](gpu-compositor.md) and
  the shared pieces in [overhaul follow-ups](overhaul-follow-ups.md) item 3.

## Open questions

1. Raising velocity mid-clip: keep the clip's length and play further into the
   footage (Vegas), or shorten it? Reverse and freeze? Remap and velocity on
   one clip?
2. Slow motion: frame repeat or blend, or optical-flow interpolation?
3. Audio under speed changes: drop, follow pitch, or keep pitch?
4. Footage and delivery: camera, codec, resolution, frame rate; a real clip
   for the benchmark; export formats.
5. Import: copy footage into the library, or reference it in place?

## Acceptance

Agreed behavior recorded in [Direction](../docs/DIRECTION.md); then phased
tasks: media import and playback, the edit, speed, Animate, effects and audio.
Each interaction joins the interaction benchmark with the heaviest real
footage.
