---
status: done
priority: normal
---

# Add the project unslop skill

Add [unslop](../.agents/skills/unslop/SKILL.md) with the user's exact content.

The skill-creator validator rejects the supplied `disable-model-invocation`
field because its allowed list excludes it. Preserve the requested field.
The file requires no scripts or other resources.

The project task/document checks and `git diff --check` passed.
