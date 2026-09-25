---
status: todo
priority: high
---

# Resolve Spectrum's packaged macOS icon appearance

Earlier Lumen/Prism icon revisions in [PR #100](https://github.com/bentsignal/spectrum/pull/100)
and [PR #101](https://github.com/bentsignal/spectrum/pull/101) were rejected
after real Dock review. Spectrum packages `assets/branding/Spectrum.icon`.
Review it in the real macOS Dock at multiple sizes. If it needs work, revise the source
composition, package the signed Spectrum app, and verify provenance and
codesign. See [prior scope](https://github.com/bentsignal/spectrum/issues/108).
