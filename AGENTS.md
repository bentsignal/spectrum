# Agent guide

Use the project-local UAV skill at `.agents/skills/uav/SKILL.md` at the start of
every project-work session. Run `uav status` before implementation, keep durable
feature work in `uav task`, record consequential decisions with `uav remember`,
and require a successful `uav closeout` before handing work back.

This repository is the Spectrum creative-suite monorepo. Applications live in
`apps/` (`apps/lumen`, `apps/prism`); app-neutral imaging behavior lives in
`crates/spectrum-imaging`; repository policy checks live in
`tools/workspace-guardrails`. Preserve each app's focused UI. Do not make one
app depend on another for behavior that belongs in a neutral Spectrum crate.

Use the `lumen` CLI for all photo and catalog automation and the `prism` CLI for
all layered-document automation. Do not edit `.lumencatalog`, `.prism`, or
legacy `.mica` JSON manually unless recovering a damaged file; the CLIs apply validation,
transactional mutation, and path checks.

Start with:

```sh
cargo run --release -p lumen-photo --bin lumen -- schema
cargo run --release -p lumen-photo --bin lumen -- --catalog <path> list
cargo run --release -p prism --bin prism -- schema
cargo run --release -p prism --bin prism -- --project <path> list
```

Every GUI mutation maps to `lumen_core::Command`. When adding a new user-facing
feature, add its core command and CLI surface before or alongside its GUI control.
Keep originals immutable and export only to user-selected destination paths.
Apply the same rule to `prism_core::Command`: its native GUI is a visual client of
the same command engine used by agents.

All Rust source files under `apps/`, `crates/`, and `tools/` must stay at or
below 1,000 lines. `workspace-guardrails` enforces this automatically. Treat
the limit as a backstop: split files by responsibility before they approach it.

## Required end-of-run validation loop

After every run that changes code, manifests, scripts, packaging, or CI, run
the complete loop below before committing **and before handing work back**:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --all-targets --locked
```

If any command fails, fix the cause and restart the complete loop from the
formatter. Continue until all three commands pass. Do not commit or hand off a
failed run. The only exception is a genuine external blocker that cannot be
fixed in the repository; record it in UAV, return the claimed task to an
appropriate non-complete state, and report the exact failing command.

For rendering or interaction performance changes, also run the affected
release benchmark with `--strict`. For packaging changes, run the affected
packaging script and verify its produced application or binary before handoff.
Only after validation succeeds should you record the outcome with `uav
remember`, resolve the task, and require a successful `uav closeout`.
