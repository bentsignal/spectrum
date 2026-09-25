---
status: deferred
priority: normal
---

# Plan stable and development Mac build tracks

The user works on a MacBook Pro while agents build Spectrum on NixOS. Manual
GitHub Actions artifact download now works and is acceptable for testing.
Developer ID signing and notarization are enabled for main pushes and manual
main builds. [Verification run 36183500119](https://github.com/bentsignal/spectrum/actions/runs/36183500119)
passed signing, notarization, stapling, Gatekeeper assessment, and a normal
Mac launch without a Privacy & Security exception. The user also confirmed
the later signed build opened normally. Pull request builds remain ad hoc.
The Apple credentials live in GitHub Actions secrets, outside the repository.

The remaining idea is a stable release track and a development track that can
select recent builds. The in-app switcher and build picker are unchosen UX,
not requirements for current manual testing. Revisit after core workspace and
library behavior settles.

## Questions to resolve

- Which commits should appear in each track, with what validation status?
- Should the Mac app select and download builds itself, or should an external
  launcher manage the installed version?
- How should installations coexist with app data and macOS signing?

## Acceptance criteria

- Define stable and development build selection, installation, and rollback.
- Implement and test the chosen flow on a Mac before marking this done.
