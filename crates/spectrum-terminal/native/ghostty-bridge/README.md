# Spectrum Ghostty terminal bridge

This directory is `spectrum-terminal`'s macOS-only, versioned dynamic bridge to
the pinned Ghostty 1.3.1 embedding API. Linux, Windows, ordinary macOS builds,
and macOS runs without explicit opt-in continue to use the portable PTY and
egui renderer.

Two independent gates are required:

1. Package either application explicitly through the pinned, private proof chain:

   ```sh
   DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer \
     bash scripts/package-prism-macos.sh --with-ghostty
   DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer \
     bash scripts/package-macos.sh --with-ghostty
   ```

   This mode requires the pinned Xcode 26.5 (17F42) through process-local
   `DEVELOPER_DIR`. The package script creates a mode-700 temporary root below
   `target/`, invokes the reviewed proof builder into that private root with the
   checksum-pinned official Zig archive, fully sealed CLT macOS 15.2 SDK data,
   and checksum-pinned `libtool` from Xcode 26.5, seals the resulting XCFramework,
   binary/header/manifest, resource tree, sentinel, and license, and immediately
   builds the bridge from a read-only snapshot in the same process chain.
   Externally prepared proof roots are rejected. The generated attestation is
   checked only as diagnostic consistency metadata; it never authorizes an
   artifact. The script then enables Cargo's `ghostty-terminal` feature and
   bundles the signed bridge, Ghostty resources, and license. Explicit Ghostty
   packages set and verify a macOS 13.0 deployment target; ordinary packages
   retain the existing macOS 11.0 minimum. Generated Ghostty archives are
   deliberately not claimed to be byte-reproducible across identical builds.

2. Launch the matching package with `PRISM_EXPERIMENTAL_GHOSTTY=1` for Prism or
   `LUMEN_EXPERIMENTAL_GHOSTTY=1` for Lumen.

If the dylib is missing, its ABI version is wrong, a required symbol cannot be
resolved, global/runtime initialization fails, or a surface cannot be created,
Spectrum reports the failure and keeps or returns the affected session to the
portable implementation. A development build may point to an explicit dylib
with `PRISM_GHOSTTY_BRIDGE` or `LUMEN_GHOSTTY_BRIDGE` set to the absolute path
of `libSpectrumGhosttyBridge.dylib`.

The Rust host extracts eframe's parent NSView only on macOS, communicates
through opaque handles, and owns teardown order. Every surface is destroyed at
most once and before the runtime. The dylib uses `RTLD_NODELETE` and is never
closed during the process, while queued main-thread callbacks are canceled by
runtime shutdown, so delayed Swift releases cannot call unmapped code or a
released Rust callback context.
The bridge converts egui's top-left logical-point rectangles to its parent
NSView coordinates and combines Spectrum visibility, active-terminal selection,
modal occlusion, window occlusion, and focus before waking Ghostty rendering.

This first production-shaped slice intentionally does not claim complete IME,
accessibility, mouse input or selection, OSC 52 approval, secure input, URLs,
notifications, search, or renderer-health behavior. Those remain runtime
acceptance work before the gate can become a default.
