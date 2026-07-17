# Spectrum

This repository is a Rust workspace for small, fast, agent-first creative tools.
The suite shares rendering primitives and automation conventions while keeping
each application's workspace focused.

The repository root is a virtual Cargo workspace. Applications live under
`apps/`, reusable Spectrum behavior under `crates/`, and repository-wide policy
checks under `tools/`. `crates/spectrum-imaging` is the first neutral shared
kernel; it owns adjustment models and app-independent pixel rendering rather
than placing those concepts inside Lumen.

`workspace-guardrails` recursively checks Rust sources under `apps/`, `crates/`,
and `tools/`; files over 1,000 lines fail both local workspace tests and CI.

## Applications

| App | Focus | Binaries |
| --- | --- | --- |
| Lumen | Photo library, RAW development, culling, presets, and batch export | `lumen`, `lumen-gui` |
| Prism | Layered canvas composition, text, masks, transforms, and image export | `prism`, `prism-gui` |

Lumen is intentionally not a layer editor, and Prism is intentionally not a
photo catalog. Opening a photo from Lumen in Prism is an explicit handoff rather
than a reason to crowd either interface.

Prism's editable document format uses the `.prism` extension. Legacy `.mica`
projects remain readable and writable. The format is an exchange
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

Prism's `from-lumen` flow is the first explicit exchange boundary: it asks the
Lumen side to develop a catalog photo, then creates a Prism project with that
result as a layer. Prism can reuse the shared imaging kernel, while Lumen remains
independent of Prism. Future handoffs should follow the same rule: exchange a
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
are named `package-<platform>`; Prism's are named `package-prism-<platform>`.
