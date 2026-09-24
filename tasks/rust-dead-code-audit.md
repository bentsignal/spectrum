---
status: deferred
priority: normal
---

# Evaluate Hawk for dead Rust code

Run a dry evaluation of Hawk against the Cargo workspace, pinning version and
configuration. Audit false positives for features, platforms, binaries, tests,
generated code, and public APIs. Remove genuinely unused code in small batches,
preserve app boundaries, run the full validation loop, and measure what changed.
See [prior scope](https://github.com/bentsignal/spectrum/issues/106).
