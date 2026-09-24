# Project documents

The current repository, its tests, and the documents below are authoritative.
Git and pull requests explain completed work. [Migration disposition](MIGRATION.md)
records how the former UAV material was curated.

| Read | Purpose |
| --- | --- |
| [Direction](DIRECTION.md) | Product intent and decisions that remain open |
| [Development](DEVELOPMENT.md) | Reproducible NixOS setup and daily commands |
| [Suite](SUITE.md) | App boundaries and shared engineering principles |
| [Prism](PRISM.md) | Prism workflow and command model |
| [Architecture](ARCHITECTURE.md) | Lumen architecture and rendering contracts |
| [CLI](CLI.md) | Lumen automation reference |
| [Agent guide](../AGENTS.md) | Working rules and required validation |
| [Tasks](../tasks/README.md) | Remaining work; one file per task |

Use `cargo run -p workspace-guardrails --bin workspace-tasks -- list` to see
active tasks. The [create-task skill](../.agents/skills/create-task/SKILL.md)
defines the file convention.
