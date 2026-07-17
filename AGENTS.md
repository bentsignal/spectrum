# Agent guide

Use the project-local UAV skill at `.agents/skills/uav/SKILL.md` at the start of
every project-work session. Run `uav status` before implementation, keep durable
feature work in `uav task`, record consequential decisions with `uav remember`,
and require a successful `uav closeout` before handing work back.

This repository is the creative-suite monorepo. Lumen is the focused photo
developer; Prism is the layered canvas editor under `apps/prism`. Preserve each
app's focused UI and put reusable imaging behavior in shared Rust core APIs.

Use the `lumen` CLI for all photo and catalog automation and the `prism` CLI for
all layered-document automation. Do not edit `.lumencatalog`, `.prism`, or
legacy `.mica` JSON manually unless recovering a damaged file; the CLIs apply validation,
transactional mutation, and path checks.

Start with:

```sh
cargo run --release --bin lumen -- schema
cargo run --release --bin lumen -- --catalog <path> list
cargo run --release -p prism --bin prism -- schema
cargo run --release -p prism --bin prism -- --project <path> list
```

Every GUI mutation maps to `lumen_core::Command`. When adding a new user-facing
feature, add its core command and CLI surface before or alongside its GUI control.
Keep originals immutable and export only to user-selected destination paths.
Apply the same rule to `prism_core::Command`: its native GUI is a visual client of
the same command engine used by agents.

Before committing, run:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
```
