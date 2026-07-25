# Prism contextual-toolbar overflow prototypes

> **Prototype only — do not merge or select a production design from this
> branch without user review.**

This branch exposes three runtime-switchable approaches to Prism's contextual
toolbar at constrained window widths. Normal launches are byte-for-byte on the
existing toolbar path. To enable the comparison in a package or development
build:

```sh
PRISM_TOOLBAR_OVERFLOW_PROTOTYPES=A /path/to/Prism.app/Contents/MacOS/prism-gui
```

The warning banner switches among all options without relaunching.
It also switches the live Prism tool among Magic Wand, Shape, Text, and
Selection so the prototypes exercise the actual controls and command paths.
No project JSON is edited by the prototype.

When native window automation cannot reach a resize border, the same banner can
constrain the real toolbar subtree to 980, 1200, or 1920 points. The equivalent
launch-time setting is:

```sh
PRISM_TOOLBAR_OVERFLOW_PROTOTYPES=A \
PRISM_TOOLBAR_PROTOTYPE_WIDTH=980 \
/path/to/Prism.app/Contents/MacOS/prism-gui
```

This is a deterministic geometry harness, not a claim that Computer Use resized
the native window.

## A · Trailing More

- Wide: unchanged inline toolbar.
- Narrow: tool identity and Tools & Actions remain fixed; every contextual
  control moves into a labeled, vertically ordered `More · N` menu.
- Strength: clearest, quietest constrained layout and easiest complete scan.
- Cost: contextual state is one activation farther away.

## B · Scroll rail

- Wide: unchanged inline toolbar.
- Narrow: one horizontal row with persistent left/right buttons and an
  always-visible scrollbar. Pointer dragging/wheel scrolling and keyboard
  focus remain available.
- Strength: preserves spatial continuity and the current one-row density.
- Cost: offscreen state is less glanceable and frequent end-to-end travel is
  slower.

## C · Adaptive wrap

- Wide: unchanged inline toolbar.
- Narrow: the same controls wrap in source/focus order onto two or three rows.
- Strength: every control remains visible with no disclosure or scrolling.
- Cost: consumes more canvas height and row breaks can shift as the window
  changes.

The environment gate and warning window are deliberate. This branch is for
comparison screenshots and hands-on review; it must remain a draft PR until the
user chooses a direction.
