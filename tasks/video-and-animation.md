---
status: in_progress
priority: high
---

# Video and animation editors

Behavior is decided in [Direction](../docs/DIRECTION.md), "Video and
animation". The user's words (2026-10-13): Spectrum takes motion next,
designed as one piece. Premiere-style editing; After Effects time remapping
and Sony Vegas velocity keyframes (Vegas is in their workflow only for
velocity); After Effects-style keyframing without 100 layers for what would
be a few tracks; hop between editing and animating a clip or a canvas; make a
canvas feel alive, go back to add text, return to animating. They record MKV
and MP4 and shoot on a Sony camera, and export MP4.

## Phases

Engine and CLI first in each; every interaction joins the interaction
benchmark with the heaviest real footage.

1. **Media in the library** (built 2026-10-13): FFmpeg decoding
   (`spectrum-media`), exact media time (`spectrum-time`), footage and audio
   copied into a content-addressed media store, posters and waveforms, CLI
   (`import`, `footage`, `audio`), automatic playback copies for heavy
   footage, and a player with sound, J/K/L, frame steps, and a typeable
   timecode. Left: hardware decoding (VideoToolbox, VA-API), a real camera
   clip from the user for the benchmark, and the user's review.
2. **The edit**: video assets with tracks and clips, cut, trim, ripple, roll,
   slip, linked sound, real-time playback, export (encode).
3. **Speed**: velocity and time remap curves, repeat past the end, reverse,
   freeze, audio that follows, frame interpolation.
4. **Animate**: a neutral keyframe model for every property; clip and canvas
   layer lanes, graph editor, animated canvases playing in videos, grouping
   clips into a canvas.
5. Effect stacks, transitions, and audio editing.

Playback at speed may need the [GPU compositor](gpu-compositor.md) and the
shared pieces in [overhaul follow-ups](overhaul-follow-ups.md) item 3.
