---
status: deferred
priority: normal
---

# Reuse the terminal dock in Lumen

Prism uses the app-neutral `spectrum-terminal` crate. Add a focused Lumen dock
with Lumen-specific catalog and live-session context; keep terminal transport
shared and avoid an app-to-app dependency. Bloom can use the same foundation
when it exists. Validate process lifecycle, focus, shortcuts, and packaging.
