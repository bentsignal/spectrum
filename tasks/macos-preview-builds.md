---
status: in_progress
priority: normal
---

# Plan Mac preview builds for remote Spectrum development

The user works in T3 Code on a MacBook Pro while agents build Spectrum on a
separate NixOS machine. They want to test that work as a native app on the Mac
without moving the AI development workload onto it. GitHub Actions already
builds macOS packages and uploads artifacts; use that as a starting point when
designing a convenient delivery flow.

The user accepts manual artifact download for now. Developer ID signing and
notarization were enabled and verified on September 25, 2026 for main pushes
and manual main builds. GitHub Actions contains all four required secrets;
`SPECTRUM_APPLE_TEAM_ID=39K6A9FP99` and
`SPECTRUM_MACOS_SIGNING_ENABLED=true`. Credentials remain outside the repository.

[Verification run 36183500119](https://github.com/bentsignal/spectrum/actions/runs/36183500119)
built commit `958d16a60cc771403536c03cf545c65806d33d85`. The macOS job signed the
app with hardened runtime and secure timestamps, received Apple's Accepted
notarization result, stapled the ticket, and uploaded `spectrum-macOS` containing
`Spectrum-macos-notarized.zip`. The downloaded ZIP passed
`codesign --verify --deep --strict --verbose=2`, `xcrun stapler validate`, and
`spctl --assess --type execute --verbose=2` on the owner's Mac; Gatekeeper reported
`source=Notarized Developer ID`. With download quarantine applied, the owner
clicked the ordinary Open confirmation and the app launched without a Privacy
& Security exception. The running executable matched the downloaded binary.
The CI format, lint, and test job also passed.

The Developer ID Application certificate belongs to team `39K6A9FP99`, expires
September 17, 2031, and has SHA-256 fingerprint
`40:EC:73:82:00:24:F8:8B:EF:04:77:6D:2E:5D:38:6E:DE:27:F3:CE:98:44:E8:69:C9:D8:85:62:9E:20:3B:D8`.
Download the notarized ZIP inside the artifact for manual installation. Pull
request builds remain ad hoc; stable/development track design is still open.

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
