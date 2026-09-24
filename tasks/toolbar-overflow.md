---
status: blocked
priority: high
---

# Choose Prism toolbar overflow behavior

The [draft A/B/C review PR](https://github.com/bentsignal/spectrum/pull/92)
contains inspectable alternatives for narrow windows: a trailing More menu,
a scroll rail, and adaptive wrapping. Its package is a review build, not the
production toolbar. This task waits for the user's design choice.

After that choice, integrate the selected behavior so every contextual control
remains reachable at supported narrow widths with pointer and keyboard access.
Keep the efficient wide layout, verify the gradient editor and nested color
picker, run the required validation and packaging checks, and obtain a fresh
review before merging. Do not merge the prototype PR as-is.
