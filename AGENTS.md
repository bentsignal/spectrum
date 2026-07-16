# Agent guide

Use the project-local UAV skill at `.agents/skills/uav/SKILL.md` at the start of
every project-work session. Run `uav status` before implementation, keep durable
feature work in `uav task`, record consequential decisions with `uav remember`,
and require a successful `uav closeout` before handing work back.

Use the `lumen` CLI for all photo and catalog automation. Do not edit
`.lumencatalog` JSON manually unless recovering a damaged file; the CLI applies
range validation, transactional import, and path checks.

Start with:

```sh
cargo run --release --bin lumen -- schema
cargo run --release --bin lumen -- --catalog <path> list
```

Every GUI mutation maps to `lumen_core::Command`. When adding a new user-facing
feature, add its core command and CLI surface before or alongside its GUI control.
Keep originals immutable and export only to user-selected destination paths.

Before committing, run:

```sh
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
```
