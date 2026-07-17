# Creative suite

This repository is a Rust workspace for small, fast, agent-first creative tools.
The suite shares rendering primitives and automation conventions while keeping
each application's workspace focused.

## Applications

| App | Focus | Binaries |
| --- | --- | --- |
| Lumen | Photo library, RAW development, culling, presets, and batch export | `lumen`, `lumen-gui` |
| Mica | Layered canvas composition, text, masks, transforms, and image export | `mica`, `mica-gui` |

Lumen is intentionally not a layer editor, and Mica is intentionally not a
photo catalog. Opening a photo from Lumen in Mica is an explicit handoff rather
than a reason to crowd either interface.

Mica's editable document format uses the `.mica` extension. It is an exchange
boundary for layered work, not a replacement for source photographs or finished
image/video exports.

## Shared principles

- Rust from command engine through native desktop UI.
- A typed `Command` boundary is the source of truth for every user mutation.
- The CLI and GUI exercise the same project and rendering behavior.
- Machine-readable schema and JSON results make every feature usable by agents.
- Originals are immutable; applications save project state and export new files.
- Release builds prioritize interaction latency, small distributions, and no web
  runtime or background service.
- Windows, macOS, and Linux remain first-class build targets.

## Sharing and exchange

Common imaging primitives live below the applications so exposure, tone, color,
crop, encoding, and related behavior do not fork into subtly different engines.
Application dependencies point toward that shared kernel, never sideways in a
cycle.

Mica's `from-lumen` flow is the first explicit exchange boundary: it asks the
Lumen side to develop a catalog photo, then creates a Mica project with that
result as a layer. Mica can reuse the shared imaging kernel, while Lumen remains
independent of Mica. Future handoffs should follow the same rule: exchange a
documented asset or project representation, keep originals immutable, and make
the operation available from both CLI and GUI.

The next planned application is one unified video and motion editor. It will
combine the parts of timeline editing and motion graphics that are useful in
this workflow instead of reproducing Premiere and After Effects as two separate
products. It should consume exported or linked suite assets through explicit
handoffs and share the same command-first automation model.

## Workspace commands

Build and test the complete suite:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --all-targets --locked
cargo build --release --workspace --bins --locked
```

Build only one application package by using its platform script. Lumen's scripts
are named `package-<platform>`; Mica's are named `package-mica-<platform>`.
