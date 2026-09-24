---
status: todo
priority: normal
---

# Plan Mac preview builds for remote Spectrum development

The user works in T3 Code on a MacBook Pro while agents build Spectrum on a
separate NixOS machine. They want to test that work as a native app on the Mac
without moving the AI development workload onto it. GitHub Actions already
builds macOS packages and uploads artifacts; use that as a starting point when
designing a convenient delivery flow.

Explore two selectable app tracks: a stable track for production releases and a
development track that lets the user find and download builds made from recent
work. The in-app track switcher and build picker are ideas, not settled UX or
release policy. Keep this planning separate from the Spectrum app combination
and GPUI rebuild so it can be discussed in another thread.

## Questions to resolve

- Which commits should produce Mac builds, and how should a build identify its
  source commit and validation status?
- Should the Mac app select and download builds itself, or should an external
  launcher manage the installed version?
- How should stable and development installations coexist with app data and
  macOS signing/notarization?

## Acceptance criteria

- Agree on a Mac test-build workflow that works from the remote NixOS machine.
- Define stable and development build selection, installation, and rollback.
- Implement and test the chosen flow on a Mac before marking this done.
