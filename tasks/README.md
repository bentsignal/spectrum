# Remaining work

Each task has its own Markdown file with `status` and `priority` frontmatter.
Run `cargo run -p workspace-guardrails --bin workspace-tasks -- list` to see
the active list. The [agent guide](../AGENTS.md) and
[create-task skill](../.agents/skills/create-task/SKILL.md) define updates.

Completed changes belong in Git and pull requests. A task can cite an old
issue or PR for context, but its current scope and acceptance criteria live
here.

## Product and engineering tasks

- [Unified Spectrum app](unified-spectrum-app.md)
- [Healing Brush](healing-brush.md)
- [Layer styles](layer-styles.md)
- [Revision lifecycle](revision-lifecycle.md)
- [GPU compositor](gpu-compositor.md)
- [macOS icons](macos-icons.md)
- [Selection acceptance](selection-user-acceptance.md)
- [Rust dead-code audit](rust-dead-code-audit.md)
- [Lumen terminal dock](lumen-terminal-dock.md)
- [Mac preview builds](macos-preview-builds.md)

## Closed decisions

- [Prism toolbar overflow prototype](toolbar-overflow.md) — canceled pending a
  broader Prism redesign.
