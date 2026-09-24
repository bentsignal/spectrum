---
status: todo
priority: high
---

# Add a nondestructive sampled Healing Brush

Clone Stamp landed in [PR #94](https://github.com/bentsignal/spectrum/pull/94).
The remaining retouch primitive is a sampled Healing Brush. Build on Prism's
paint command model and freeze authenticated source and destination context at
the expected parent revision. Later source edits must not change committed
output; original images stay immutable.

One drag creates one revision and cancel creates none. Provide core command,
CLI/schema, GUI, persistence, transfer, preview/export/reopen, undo/redo, and
live collaboration parity. Use bounded deterministic processing in
`spectrum-imaging`; cover large canvases with strict benchmarks and a packaged
app check. Prior planning is in [closed issue #102](https://github.com/bentsignal/spectrum/issues/102).
