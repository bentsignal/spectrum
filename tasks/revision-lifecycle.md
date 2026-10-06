---
status: todo
priority: high
---

# Complete creative history and collaboration lifecycle

`spectrum-document` keeps every asset's revision tree with people's and agents'
sessions; the CLI exposes `history` and `history-jump`. The desktop has no
history view yet. Build one the user asked for: a tree of each asset's history
with previews, named checkpoints, clear human and agent attribution, and
discoverable alternate futures, where clicking a revision moves to it.

Then define asset reachability and provide a dry-run retention planner before
any compaction. Keep history permanent by default until the user explicitly
approves pruning. Validate recovery, bounded replay, and concurrent sessions.
See [product direction](../docs/DIRECTION.md) and
[prior scope](https://github.com/bentsignal/spectrum/issues/103).
