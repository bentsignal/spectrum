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

The user has run the Spectrum macOS CI artifact and accepts manual download
for now. They have Apple Developer Program membership. The current artifact
uses an ad hoc signature and needs a macOS opening exception.

In September 2026, the user said that repeating the macOS opening exception
for development builds is becoming tiresome and wants signing addressed soon.
The current package script signs `Spectrum.app` ad hoc (`codesign --sign -`),
and the repository has no signing or notarization credentials in GitHub
Actions. The next delivery improvement is to sign all bundled code
with a Developer ID Application identity, enable hardened runtime and secure
timestamps, submit the distribution to Apple's notary service, staple the
ticket, and verify the finished artifact on a Mac. The account owner will
need to provide a Developer ID certificate with private key and notarization
credentials through protected CI secrets; the Apple Developer membership
alone does not make those available to the runner. Keep the current ad hoc
build usable until the signed path has been verified end to end. The signing
scripts and CI gate are ready; main builds use them only when the owner sets
`SPECTRUM_MACOS_SIGNING_ENABLED=true` after configuring secrets. Missing
credentials then fail the build. Verify the notarized artifact on a Mac.

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
