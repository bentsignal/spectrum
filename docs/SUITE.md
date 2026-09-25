# Spectrum

This repository is a Rust workspace for one fast, agent-first creative desktop
app. Its photo and canvas workspaces keep focused editing interfaces while
sharing rendering primitives and automation conventions.

The repository root is a virtual Cargo workspace. Applications live under
`apps/`, reusable Spectrum behavior under `crates/`, and repository-wide policy
checks under `tools/`. `crates/spectrum-imaging` is the first neutral shared
kernel; it owns adjustment models and app-independent pixel rendering rather
than placing those concepts inside Lumen.

`workspace-guardrails` recursively checks Rust sources under `apps/`, `crates/`,
and `tools/`; files over 1,000 lines fail both local workspace tests and CI.

## Applications

| Workspace | Focus | Binaries |
| --- | --- | --- |
| Photos (Lumen engine) | Photo library, RAW development, culling, presets, and batch export | `spectrum-gui`, `lumen` |
| Canvas (Prism engine) | Layered canvas composition, text, masks, transforms, and image export | `spectrum-gui`, `prism` |

The current photo editor is not a layer editor, and the current canvas editor
is not a photo catalog. Spectrum switches between them in one window; the
current `from-lumen` handoff still creates a rendered canvas layer. These
boundaries describe today's implementation, not the intended app-managed
creative library.

Prism's current editable document format uses the `.prism` extension. Legacy
`.mica` projects remain readable and writable today. Neither format is a
product-level compatibility requirement for Spectrum's future storage model.

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

The intended destination adds video and audio workspaces. The existing Lumen
and Prism engines provide the current photo and canvas workspaces. See
[direction](DIRECTION.md) for the app-managed library and the still-open
questions about cross-workspace assets.

## Workspace commands

Build and test the complete suite:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --all-targets --locked
cargo build --release --workspace --bins --locked
```

Build the Spectrum application package with `package-spectrum-<platform>`.
