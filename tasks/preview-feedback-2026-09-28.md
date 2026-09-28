---
status: in_progress
priority: high
---

# Address the 2026-09-28 preview review

The user reviewed build `77f2dbe`. They approved always-rounded previews,
Color's six sections, and Command+number for modes. Done since (see Git):
instant in-memory image edits, live canvas drags, grabbable curve endpoints,
no grid zoom, an export dialog (format, quality, size), Option+1 to 6 for sections, Command+K arrows, and a macOS
title-bar fix. Design rules are in [workflow preferences](../docs/design/workflow.md).

Remaining:
1. Foreground and background colors for new layers, with a real color picker.
2. Canvas size by exact pixels, likely in its own popover.
3. Mac check: instant edits, live drags, no zoom on quick title-row clicks.
4. Edit globally saves before re-rendering; Command+K does not scroll to its row.
