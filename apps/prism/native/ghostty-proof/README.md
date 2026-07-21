# Prism Ghostty proof of life

This is a standalone macOS harness for testing Ghostty's full embedded terminal
surface before Prism adopts any production integration. It does not change the
Prism executable, Cargo manifests, or the portable `spectrum-terminal` backend.

The proof exercises the architectural unknowns that parser-only
`libghostty-vt` cannot answer:

- a Ghostty-owned shell and PTY;
- Metal rendering into a caller-owned AppKit `NSView`;
- process-level `ghostty_app_t` wake/tick callbacks;
- surface focus, point/backing-pixel resize, AppKit window occlusion, and close
  cleanup;
- basic physical-key and text input; and
- the generated XCFramework and resource layout used by an app bundle.

It intentionally does not claim production behavior. IME composition,
accessibility, complete mouse reporting and selection, rich clipboard types,
OSC 52 approval UI, secure input, URLs, notifications, renderer-health UI,
tabs, and eframe OpenGL/Metal composition remain TODOs.

## Pinned inputs

[`ghostty-proof.lock`](../../../../packaging/prism/macos/ghostty-proof.lock)
is the reviewed contract. It pins:

- official Ghostty `v1.3.1`, annotated tag object
  `22efb0be2bbea73e5339f5426fa3b20edabcaa11`, peeled source commit
  `332b2aefc6e72d363aa93ab6ecfc86eeeeb5ed28`, and the SHA-256 of Ghostty's
  official source release;
- official Zig 0.15.2 macOS archives and SHA-256 values for compatible arm64
  SDKs and native x86_64 hosts, plus the exact Homebrew formula and path used
  by the affected-arm64-SDK compatibility route described below;
- macOS 13.0 as the minimum deployment target; and
- the expected `GhosttyKit.xcframework`, resources directory, and terminfo
  sentinel paths.

No downloaded source or generated binary is checked into the repository.
Everything is placed below the ignored `target/ghostty-proof/` directory.
The tag object and peeled commit are provenance metadata. Because the official
release archive contains no Git-object manifest, its verified SHA-256—not an
unprovable archive-to-commit equivalence—is the downloaded source trust anchor.

### Xcode 26.4 and newer on arm64

Official Zig 0.15.2 predates Xcode 26.4's change from an `arm64-macos` root
target in `libSystem.tbd` to an `arm64e-macos` root target. Its native arm64
build runner consequently fails before Ghostty's build graph starts; this is
tracked as [Ghostty #11991](https://github.com/ghostty-org/ghostty/issues/11991)
and fixed in Zig 0.16 (Zig PR #31673), without a Zig 0.15 backport.

The proof remains pinned to Ghostty's required Zig 0.15.2. On an arm64 Mac, the
script inspects only the root target block of the selected SDK's
`libSystem.tbd`. If that block lacks `arm64-macos`, it requires Homebrew's
patched `zig@0.15` bottle at `/opt/homebrew/opt/zig@0.15/bin/zig` and verifies
that it reports exactly 0.15.2. This is the route recommended in Ghostty
#11991. The official x86_64 archive is deliberately not used under Rosetta on
Xcode 27 because that combination fails while building libc++. The script only
consumes an already-installed bottle: it does not install Homebrew software or
build Zig from source. Compatible arm64 SDKs and native x86_64 hosts continue
to use the checksummed official archives. Unlike those archives, the local
Homebrew bottle is trusted input rather than a checksum-pinned artifact; the
script verifies the formula prefix, resolved keg binary, exact reported Zig
version, and `poured_from_bottle` installation receipt before downloading
Ghostty.

## Build

Requirements:

- macOS 13 or newer;
- full Xcode selected by `xcode-select`, with macOS SDK, iOS SDK, Swift, and the
  Metal Toolchain component installed. Before any download, the script runs the
  non-compiling `xcrun --sdk macosx metal -v` availability probe; if it fails,
  install the component with `xcodebuild -downloadComponent MetalToolchain`;
- Homebrew's exact `zig@0.15` bottle when building on arm64 against an SDK whose
  root `libSystem.tbd` advertises `arm64e-macos` but not `arm64-macos`; install
  it separately with `brew install --force-bottle zig@0.15`;
- `gettext`/`msgfmt`; and
- network access to the HTTPS URLs in the lock file.

From the repository root:

```sh
bash scripts/build-prism-ghostty-macos.sh
```

The script is noninteractive. It verifies every downloaded archive before
extraction, uses Ghostty's required Zig toolchain (with the narrow
Homebrew-patched route above), limits the Ghostty build to two jobs, stages a
SwiftPM build against the generated XCFramework, copies Ghostty's resources and
license into the proof app, and applies an ad-hoc local signature. It does not
launch the app.

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
5. Minimizing, hiding, and restoring the window suspends and resumes Ghostty
   visibility without losing the live shell.
6. Closing an idle shell exits cleanly; closing with a foreground process asks
   before stopping it.
7. Activity Monitor shows no runaway idle or occluded repaint loop.

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
