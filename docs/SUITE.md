# Spectrum

Spectrum is one native Rust creative app with focused image and canvas views.
Applications live under `apps/`, shared behavior under `crates/`, and repository
policy checks under `tools/`. Rust sources have a 1,000-line maximum.

## Shared library

`crates/spectrum-library` owns stable UUID asset identities, extensible type
identifiers, and typed dependency edges. `apps/spectrum/src/library` adapts the
existing image and canvas command engines to that model. Neutral crates do not
depend on either application. `spectrum-imaging` owns shared pixel behavior.

The default library is Spectrum's application-data directory plus `Library`.
`SPECTRUM_LIBRARY` or CLI `--library` selects an isolated library for automation.
The SQLite index maps editable identities to internal durable `.spectrum`
documents and image items. It is authoritative and must be backed up. Engine
revision stores retain immutable originals and history. `previews/` is disposable.

Canvas raster layers carry a live image asset ID alongside derived pixel paths.
The desktop resolves changed images on a background worker; exports resolve
current image edits before rendering. Dependency traversal supports transitive
updates; type checks and cycle rejection apply when indexing references. Only
images and canvases have editors today. Asset types alone do not add an editor.

Independent image copies have distinct identities and revision stores. Canvas
copies recursively copy their referenced images, preserving shared references
inside the copy. Copies start new revision trees at the copied state; original
revision trees stay intact. Placement transforms remain canvas-local.

For backup and portability, close Spectrum and other library writers, then copy
**the entire library directory**, including the SQLite index and any journal
files. Restore it as a unit and open with `SPECTRUM_LIBRARY`. Do not reconstruct
identity by deleting the index. Existing engine recovery storage handles failed
publication; resolve any reported publication error before making a backup.
Managed backup/restore UI and history browsing remain future work.

## Commands and desktop

`spectrum library` lists assets; `spectrum images import <paths...>` imports;
`spectrum canvas new <name>` creates a canvas. `spectrum canvas place <canvas-id>
<image-id>` creates a live reference. `spectrum images adjust <id> <patch-json>`
edits an image; `spectrum copy <id>` makes independent content. Both domains have
`list`, `command <id> <engine-command-json>`, and `export <id> <path>` operations.
The desktop's Library menu opens assets; Place on canvas and Edit image connect
existing editing views. Independent copy creates separately editable content.

Library CLI edits use the authenticated live host when the document is open.
All editor commands, schemas, benchmarks, and collaboration tools now live under
`spectrum images` and `spectrum canvas`; packages ship no separate engine CLIs.
Advanced commands select `--asset` or `--document`; see [CLI](CLI.md) for sessions.
The GPUI rewrite follows validation of these interactions, per [direction](DIRECTION.md).

## Validation

Run `cargo fmt --all -- --check`, then
`cargo clippy --workspace --all-targets --locked -- -D warnings`, then
`cargo test --workspace --all-targets --locked`. Build release packages with
`scripts/package-spectrum-<platform>`; [development](DEVELOPMENT.md) covers NixOS.
