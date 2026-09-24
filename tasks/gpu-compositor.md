---
status: todo
priority: high
---

# Build a native GPU Prism blend compositor

The current CPU exact region path is the pixel oracle. Design a viewport and
tile-aware native GPU path for deterministic blend modes, clipping, and inverted
masks. Keep transformations responsive above 4096 px and bound memory to visible
work. Compare pixels with core export fixtures, test invalidation across masks,
styles, transforms, and revisions, and run strict interaction/render benchmarks
plus packaged-app verification. See
[prior scope](https://github.com/bentsignal/spectrum/issues/105).
