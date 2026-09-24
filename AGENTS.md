# Agent guide

Read [docs/INDEX.md](docs/INDEX.md) for the authoritative project documents.
Keep durable work in one Markdown file per task under [tasks/](tasks/), using
the project-local `create-task` skill. List and validate tasks with
`cargo run -p workspace-guardrails --bin workspace-tasks -- list`.
Record lasting product decisions in [docs/DIRECTION.md](docs/DIRECTION.md)
or the owning architecture document. Git and pull requests retain completed
work history.

## Source control and local build space

Own source control through handoff. At the start of a run, inspect the branch,
worktree, and remote state. For completed repository changes, run validation,
commit on `main`, push to `origin/main`, and verify the pushed commit matches
the remote before handing work back. Keep related work on a review branch when
an open PR or an explicit review gate requires it; finish that review and merge
before reporting the work as landed on `main`. Never overwrite unrelated
changes or force-push `main`.

Check build-space usage before and after substantial builds with `du -sh target`
and the build directories in active Git worktrees. Reuse build output where
practical and serialize local Rust builds and tests. If generated output grows
by 10 GiB during a run or the repository's build directories exceed 40 GiB in
total, identify the largest targets and clean obsolete outputs before handoff.
After a PR closes, remove its unneeded build output and temporary packages;
keep the current working build and any package still needed for review or user
testing. Verify exact paths and worktree status before cleanup. Never remove
source, uncommitted changes, user project files, or the only copy of a review
artifact. Remove only verified generated directories; moving them to Trash
does not reclaim space until they are deleted there. Confirm the reclaimed
space afterward.

This repository is the Spectrum creative-suite monorepo. Applications live in
`apps/` (`apps/lumen`, `apps/prism`); app-neutral imaging behavior lives in
`crates/spectrum-imaging`; repository policy checks live in
`tools/workspace-guardrails`. Preserve each app's focused UI. Do not make one
app depend on another for behavior that belongs in a neutral Spectrum crate.

Use the `lumen` CLI for all photo and catalog automation and the `prism` CLI for
all layered-document automation. Do not edit `.lumen`, legacy `.lumencatalog`, `.prism`, or
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
fixed in the repository; record it in the relevant task file and report the
exact failing command.

For rendering or interaction performance changes, also run the affected
release benchmark with `--strict`. For packaging changes, run the affected
packaging script and verify its produced application or binary before handoff.
Only after validation succeeds should you record the outcome in the relevant
task and update its status.
