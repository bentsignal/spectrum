---
status: todo
priority: high
---

# Resolve packaged macOS icon appearance

The merged optical scaling in [PR #100](https://github.com/bentsignal/spectrum/pull/100)
was later rejected by the user. A native Icon Composer revision in
[closed PR #101](https://github.com/bentsignal/spectrum/pull/101) was also
blocked by a real Dock review showing an oversized footprint. Revisit the
source composition, build both exact `.icon` packages with `actool`, package
Lumen and Prism, and inspect distinctly identified apps in the real Dock at
multiple sizes. Verify provenance and codesign. Do not infer success from
asset bounds alone. See [prior scope](https://github.com/bentsignal/spectrum/issues/108).
