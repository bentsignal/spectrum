---
name: create-task
description: Use when the user asks to create a task.
---

# Create a task

Store one task per Markdown file in `tasks/`, using a short, human-readable
kebab-case filename. Search existing task files before creating one so the same
work is not recorded twice.

Every task starts with exactly this frontmatter:

```yaml
---
status: todo
priority: normal
---
```

Allowed statuses are `todo`, `in_progress`, `blocked`, `deferred`, `done`, and
`canceled`. Allowed priorities are `low`, `normal`, `high`, and `urgent`. Put the
task title, context, dependencies, and acceptance criteria in the Markdown body,
not in frontmatter. Update the existing file when status or scope changes.

Run `cargo run -p workspace-guardrails --bin workspace-tasks -- list` after
creating or updating a task. The workspace package validates task frontmatter,
Markdown links, and documentation budgets during the normal test loop.
