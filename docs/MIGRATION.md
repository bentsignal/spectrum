# UAV migration disposition

On 2026-09-24, the Spectrum UAV export contained 1,003 source records:
185 intent notes, 477 ordinary notes, 231 tasks, one UAV request, and 109
source-project/worktree records. The export generated 900 temporary files,
including indexes; file count is not record count. Every record was classified
in a temporary item-level ledger and checked against current code and docs,
Git history, merged pull requests, the then-open PR #92, and closed issues.

| Disposition | Records | Result |
| --- | ---: | --- |
| Merged | 59 | 56 intent notes and one broad task into [direction](DIRECTION.md) or existing docs; two review findings into tasks |
| Moved | 14 | Remaining task records consolidated into 10 [task files](../tasks/README.md) |
| Superseded | 15 | Old naming, architecture, build assumptions, and the August instruction to stay on UAV |
| Omitted | 915 | Completed work, validation journals, transient coordination, duplicated implementation facts, and worktree metadata recoverable from code/Git/PRs |

## Contradictions and decisions

- An August 2026 intent note instructed agents to keep UAV after a previous
  migration was rolled back. The current user direction supersedes it. The
  old GitHub issues #102–#109 remain closed; their useful scope is now in
  repository task files.
- Older Mica and Flux names were superseded by Prism and Bloom. Bloom is a
  confirmed future app name, with no implementation task yet.
- Older Lumen catalog and benchmark notes conflict with the current portable
  project and published performance contracts. Current code and
  [CLI documentation](CLI.md) govern those mechanics.
- The merged macOS icon scaling was later rejected by the user, and a follow-up
  Icon Composer PR was closed after a failed Dock review. The appearance is
  unresolved and tracked in [macOS icons](../tasks/macos-icons.md).
- [Prism toolbar PR #92](https://github.com/bentsignal/spectrum/pull/92)
  was a review prototype. The user subsequently closed it without selecting
  A/B/C, pending a broader Prism redesign. The
  [task record](../tasks/toolbar-overflow.md) captures that decision.

No other unresolved source conflict was found. Historical UAV source records
were left intact. Repository docs and task files now hold active project
context; [the index](INDEX.md) identifies their owners.
