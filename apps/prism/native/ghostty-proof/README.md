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
tabs, actual eframe-owned child-`NSView` lifecycle and input routing, and
eframe OpenGL/Ghostty Metal composition remain TODOs.

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

### Affected arm64 SDKs (validated with Xcode 26.5)

Official Zig 0.15.2 predates Xcode 26.4's change from an `arm64-macos` root
target in `libSystem.tbd` to an `arm64e-macos` root target. Its native arm64
build runner consequently fails before Ghostty's build graph starts; this is
tracked as [Ghostty #11991](https://github.com/ghostty-org/ghostty/issues/11991)
and fixed in Zig 0.16 (Zig PR #31673), without a Zig 0.15 backport.

The proof remains pinned to Ghostty's required Zig 0.15.2. On an arm64 Mac, the
script inspects only the root target block of the active SDK's
`libSystem.tbd`. If that block lacks `arm64-macos`, it requires Homebrew's
patched `zig@0.15` bottle at `/opt/homebrew/opt/zig@0.15/bin/zig` and verifies
that it reports exactly 0.15.2. This is the route recommended in Ghostty
#11991. The official x86_64 archive is deliberately not used under Rosetta
because that workaround fails while building libc++ on newer SDKs.

The Homebrew Zig build also needs
`/opt/homebrew/opt/llvm@20/bin/llvm-libtool-darwin`, installed as Zig's
`llvm@20` dependency. Apple's `libtool` drops Zig archive members that are not
8-byte aligned, leaving `libghostty.a` without required public symbols. The
script places a build-local `libtool` shim ahead of `PATH`, backed by LLVM's
tool, then verifies that `_ghostty_init` and `_ghostty_app_new` exist before
Swift linking. It does not change the machine-wide `PATH`.

The user-tested proof was built on Apple Silicon with stable Xcode 26.5
(17F42), macOS SDK 26.5, Homebrew `zig@0.15` 0.15.2, and `llvm@20` 20.1.8.
Typing and rapid resizing passed the requested smoke test. The build selected
stable Xcode only for that process with `DEVELOPER_DIR`; the globally selected
Xcode 27 beta was not changed. Compatible arm64 SDKs and native x86_64 hosts
continue to use the checksummed official Zig archives. Unlike those archives,
the local Homebrew bottles are trusted input rather than checksum-pinned
artifacts; the script verifies Zig's formula prefix, resolved keg binary,
exact reported version, and `poured_from_bottle` installation receipt before
downloading Ghostty. It separately verifies LLVM's locked formula and version,
formula prefix, resolved keg binary, bottle receipt, arm64 architecture, and
reported tool version.

## Build

Requirements:

- macOS 13 or newer;
- full Xcode selected for the process through `DEVELOPER_DIR` or globally with
  `xcode-select`, with macOS SDK, iOS SDK, Swift, and the Metal Toolchain
  component installed. Before any download, the script runs the non-compiling
  `xcrun --sdk macosx metal -v` availability probe; if it fails, install the
  component with `xcodebuild -downloadComponent MetalToolchain`;
- Homebrew's exact `zig@0.15` bottle when building on arm64 against an SDK whose
  root `libSystem.tbd` advertises `arm64e-macos` but not `arm64-macos`; install
  it separately with `brew install --force-bottle zig@0.15`. Its `llvm@20`
  bottle dependency must provide `llvm-libtool-darwin` at the locked path;
- `gettext`/`msgfmt`; and
- network access to the HTTPS URLs in the lock file.

From the repository root:

```sh
DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer \
  bash scripts/build-prism-ghostty-macos.sh
```

Omit `DEVELOPER_DIR` when the intended full Xcode is already selected globally.
The script never calls `xcode-select --switch` and does not alter global Xcode
selection.

The script is noninteractive. It verifies every downloaded archive before
extraction. Downloads and the official Zig toolchain may remain cached, but
every invocation removes and re-extracts the exact Ghostty source tree and
clears its exact install prefix before building. This prevents a prior build's
generated XCFramework or resources from satisfying the current build's output
checks. The script uses Ghostty's required Zig toolchain (with the narrow
Homebrew-patched route above), limits the Ghostty build to two jobs, stages a
SwiftPM build against the generated XCFramework, copies Ghostty's resources
and license into the proof app, and applies an ad-hoc local signature. It does
not launch the app.

Expected output:

```text
target/ghostty-proof/dist/PrismGhosttyProof.app
```

Ghostty installs a `share/` tree. The script copies the contents of that tree
directly into `Contents/Resources`; it does not add an extra wholesale
`Resources/ghostty/` wrapper. The packaged sentinel is therefore:

```text
PrismGhosttyProof.app/Contents/Resources/terminfo/78/xterm-ghostty
```

Ghostty-owned subtrees retain their upstream names, so themes remain under
`Contents/Resources/ghostty/themes/`.

## Run and test manually

After the build succeeds:

```sh
open target/ghostty-proof/dist/PrismGhosttyProof.app
```

Acceptance status for the user-tested Xcode 26.5 build:

- [x] A shell prompt accepted `ls`, Return, and rendered its output.
- [x] Rapid window resize remained visually attached without an observed stale
  backing-scale artifact.
- [ ] Arrow-key input and Command-C/Command-V clipboard behavior are unverified.
- [ ] Moving the window between Retina-scaled displays and checking scale
  updates is unverified.
- [ ] Focus-on-click while command output continues is unverified.
- [ ] Minimize, hide, and restore visibility behavior is unverified.
- [ ] Idle close and the foreground-process confirmation path are unverified.
- [ ] Activity Monitor inspection for runaway idle or occluded repaint is
  unverified.

Only the two checked items are completed evidence. The remaining entries are a
pending manual acceptance checklist, not proof claims.

Do not use this harness to evaluate IME, VoiceOver, terminal selection, OSC 52,
or Prism/eframe child-view and overlay behavior; those paths are deliberately
incomplete.

## Source-only checks

These checks do not download or build dependencies:

```sh
bash -n scripts/build-prism-ghostty-macos.sh
plutil -lint apps/prism/native/ghostty-proof/Info.plist
DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer \
  xcrun swift-format lint --strict --recursive \
  apps/prism/native/ghostty-proof/Package.swift \
  apps/prism/native/ghostty-proof/Sources
DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer \
  xcrun swiftc -frontend -parse \
  apps/prism/native/ghostty-proof/Package.swift \
  apps/prism/native/ghostty-proof/Sources/PrismGhosttyProof/*.swift
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
