# Project documents

Resume UI work from the [handoff](design/handoff.md). These documents and tests are authoritative.
Git and pull requests explain completed work.

| Read | Purpose |
| --- | --- |
| [Direction](DIRECTION.md) | Product intent, decisions, and open questions |
| [Architecture](ARCHITECTURE.md) | Crates, the library, durable documents, and engines |
| [CLI](CLI.md) | Spectrum automation reference |
| [Development](DEVELOPMENT.md) | Reproducible NixOS setup and daily commands |
| [Agent guide](../AGENTS.md) | Working rules and required validation |
| [Tasks](../tasks/README.md) | Remaining work; one file per task |

Use `cargo run -p workspace-guardrails --bin workspace-tasks -- list` to see
active tasks. The [create-task skill](../.agents/skills/create-task/SKILL.md)
defines the file convention.
