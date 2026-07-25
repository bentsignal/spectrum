# Prism contextual-toolbar overflow prototypes

> **Prototype only — do not merge or select a production design from this
> branch without user review.**

This branch exposes three runtime-switchable approaches to Prism's contextual
toolbar at constrained window widths. The comparison harness is visible by
default on this review-only branch so a tester cannot accidentally launch an
apparently ordinary Prism build that hides the options. Launch-time selection
remains available:

```sh
PRISM_TOOLBAR_OVERFLOW_PROTOTYPES=A /path/to/Prism.app/Contents/MacOS/prism-gui
```

The warning banner switches among all options without relaunching.
It also switches the live Prism tool among Magic Wand, Shape, Text, and
Selection so the prototypes exercise the actual controls and command paths.
No project JSON is edited by the prototype.

Set `PRISM_TOOLBAR_OVERFLOW_PROTOTYPES=off` only when comparing against the
unchanged production toolbar path.

When native window automation cannot reach a resize border, the same banner can
model the real responsive decision at 980, 1200, or 1920 points while keeping
rendering inside the native review window. The equivalent launch-time setting
is:

```sh
PRISM_TOOLBAR_OVERFLOW_PROTOTYPES=A \
PRISM_TOOLBAR_PROTOTYPE_WIDTH=980 \
/path/to/Prism.app/Contents/MacOS/prism-gui
```

This is a deterministic geometry harness, not a claim that Computer Use resized
the native window.

The current multi-stop gradient editor is deliberately never embedded into the
horizontal toolbar, scroll rail, or wrapped rows. A compact, truthful
`Gradient · Kind · N stops` control opens a bounded floating editor panel.
That panel retains geometry, spread, every stop, and each stop's nested visual
color picker. Closed vector paths receive the same access; open paths do not
offer fill controls.

## Dedicated review package

Build the review app with:

```sh
bash scripts/package-prism-toolbar-review-macos.sh
```

The result is `target/dist/Prism Toolbar Review.app`, with display name
`Prism Toolbar Review` and bundle identifier
`com.bentsignal.prism.toolbar-review`. The normal `Prism.app` identity remains
unchanged. The distinct identity prevents macOS from resolving this prototype
to another installed or worktree Prism bundle.

## A · Trailing More

- Wide: unchanged inline toolbar.
- Narrow: tool identity and Tools & Actions remain fixed; every contextual
  control moves into a labeled, vertically ordered More menu. Gradient access
  is a nested, explicitly labeled floating editor.
- Strength: clearest, quietest constrained layout and easiest complete scan.
- Cost: contextual state is one activation farther away.

## B · Scroll rail

- Wide: unchanged inline toolbar.
- Narrow: one horizontal row with persistent left/right buttons and an
  always-visible scrollbar. Pointer dragging/wheel scrolling and keyboard
  focus remain available. The gradient summary remains compact and opens the
  same editor panel.
- Strength: preserves spatial continuity and the current one-row density.
- Cost: offscreen state is less glanceable and frequent end-to-end travel is
  slower.

## C · Adaptive wrap

- Wide: unchanged inline toolbar.
- Narrow: the same controls wrap in source/focus order onto two or three rows.
- The full gradient editor remains in its floating panel instead of expanding a row
  into a clipped vertical inspector.
- Strength: every control remains visible with no disclosure or scrolling.
- Cost: consumes more canvas height and row breaks can shift as the window
  changes.

The environment gate and warning window are deliberate. This branch is for
comparison screenshots and hands-on review; it must remain a draft PR until the
user chooses a direction.
