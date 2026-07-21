# Prism Ghostty proof of life

This is a standalone macOS harness for testing Ghostty's full embedded terminal
surface before Prism adopts any production integration. It does not change the
Prism executable, Cargo manifests, or the portable `spectrum-terminal` backend.

The proof exercises the architectural unknowns that parser-only
`libghostty-vt` cannot answer:

- a Ghostty-owned shell and PTY;
- Metal rendering into a caller-owned AppKit `NSView`;
- process-level `ghostty_app_t` wake/tick callbacks;
- surface focus, point/backing-pixel resize, and close cleanup;
- basic physical-key and text input; and
- the generated XCFramework and resource layout used by an app bundle.

It intentionally does not claim production behavior. IME composition,
accessibility, complete mouse reporting and selection, rich clipboard types,
OSC 52 approval UI, secure input, URLs, notifications, renderer-health UI,
tabs, and eframe OpenGL/Metal composition remain TODOs.

## Pinned inputs

[`ghostty-proof.lock`](../../../../packaging/prism/macos/ghostty-proof.lock)
is the reviewed contract. It pins:

- official Ghostty `v1.3.1`, tag revision
  `22efb0be2bbea73e5339f5426fa3b20edabcaa11`, and the SHA-256 of Ghostty's
  official source release;
- official Zig 0.15.2 macOS archives and SHA-256 values for arm64 and x86_64;
- macOS 13.0 as the minimum deployment target; and
- the expected `GhosttyKit.xcframework`, resources directory, and terminfo
  sentinel paths.

No downloaded source or generated binary is checked into the repository.
Everything is placed below the ignored `target/ghostty-proof/` directory.

## Build

Requirements:

- macOS 13 or newer;
- full Xcode selected by `xcode-select`, with macOS SDK, iOS SDK, Swift, and the
  Metal toolchain installed;
- `gettext`/`msgfmt`; and
- network access to the HTTPS URLs in the lock file.

From the repository root:

```sh
bash scripts/build-prism-ghostty-macos.sh
```

The script is noninteractive. It verifies both downloads before extraction,
uses Ghostty's required Zig toolchain, limits the Ghostty build to two jobs,
stages a SwiftPM build against the generated XCFramework, copies Ghostty's
resources and license into the proof app, and applies an ad-hoc local signature.
It does not launch the app.

Expected output:

```text
target/ghostty-proof/dist/PrismGhosttyProof.app
```

The packaged resource sentinel must be:

```text
PrismGhosttyProof.app/Contents/Resources/ghostty/terminfo/78/xterm-ghostty
```

## Run and test manually

After the build succeeds:

```sh
open target/ghostty-proof/dist/PrismGhosttyProof.app
```

Check only the proof claims:

1. A shell prompt appears and accepts ordinary text, Return, arrows, Command-C,
   and Command-V.
2. Rapid window resize keeps terminal content attached to the view without
   stale backing-scale artifacts.
3. Moving the window between Retina-scaled displays updates rendering scale.
4. The surface gains focus on click and continues rendering command output.
5. Closing an idle shell exits cleanly; closing with a foreground process asks
   before stopping it.
6. Activity Monitor shows no runaway idle repaint loop.

Do not use this harness to evaluate IME, VoiceOver, terminal selection, OSC 52,
or Prism/egui overlay behavior; those paths are deliberately incomplete.

## Source-only checks

These checks do not download or build dependencies:

```sh
bash -n scripts/build-prism-ghostty-macos.sh
plutil -lint apps/prism/native/ghostty-proof/Info.plist
git diff --check
```

The Swift sources import `GhosttyKit`, so a meaningful Swift typecheck requires
the generated XCFramework. The build script performs that compilation after
the verified Ghostty build.

## Cleanup

From a verified Spectrum repository root, confirm the target first and then
remove only the ignored proof directory:

```sh
test -f Cargo.toml && test -d target/ghostty-proof
rm -rf -- target/ghostty-proof
```

No tracked file is generated or modified by the build.

## Provenance and maintenance warning

Ghostty's full embedding header in v1.3.1 explicitly says it is not yet a
general-purpose stable API. This harness is pinned because structs, callbacks,
actions, artifact layout, link requirements, and host-view behavior may change
without compatibility guarantees. Upgrade the lock, bridge, and manual test
evidence together; never change only the download URL.
