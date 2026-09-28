---
status: in_progress
priority: high
---

# Address the 2026-09-28 preview review

The user reviewed build `77f2dbe`. They approved always-rounded previews,
Color's six sections, and Command+number for modes. Done since (see Git):
instant in-memory image edits, live canvas drags, grabbable curve endpoints,
no grid zoom, an export dialog, a color picker with foreground and background
colors (text takes the foreground, shapes the background; confirm with the
user), a canvas size dialog, Option+1 to 6 for sections, Command+K arrows and scrolling, and a macOS
title-bar fix. Design rules are in [workflow preferences](../docs/design/workflow.md).

Remaining:
1. Mac check: instant edits, live drags, no zoom on quick title-row clicks.
2. Edit globally saves each change before the canvas re-renders.
