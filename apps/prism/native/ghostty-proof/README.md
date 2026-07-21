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
- official Zig 0.15.2 macOS archives and SHA-256 values;
- the root-owned CLT macOS 15.2 SDK path, identity, key-file hashes, and a
  complete hash of its file bytes, topology, modes, and safe symlink targets;
- the universal macOS target, macOS 13.0 minimum deployment target, Prism
  bridge ABI, reviewed Xcode version/build, and exact selected-Xcode `libtool`
  SHA-256; and
- the expected `GhosttyKit.xcframework`, resources directory, and terminfo
  sentinel paths.

No downloaded source or generated binary is checked into the repository.
Everything is placed below the ignored `target/ghostty-proof/` directory.
The tag object and peeled commit are provenance metadata. Because the official
release archive contains no Git-object manifest, its verified SHA-256—not an
unprovable archive-to-commit equivalence—is the downloaded source trust anchor.

The production proof path accepts only a checksum-pinned official Zig archive.
Xcode 26.5's root `libSystem.tbd` advertises `arm64e-macos` but not
`arm64-macos`, which official arm64 Zig 0.15.2 cannot parse. A reviewed
package-private `xcrun` shim therefore returns the fully sealed CLT macOS 15.2
SDK only for Zig's exact macOS SDK-discovery call. Every other `xcrun` request,
including iOS SDK, Metal, and Swift discovery, delegates to Xcode 26.5.
Ghostty invokes bare `libtool` while combining its static dependencies, so the
proof also exposes the exact checksum-pinned `libtool` inside Xcode 26.5. Every
shim and input is verified before and after the build. The proof never executes
a machine-installed Zig or LLVM toolchain, and does not execute CLT compilers.

## Build

Requirements:

- macOS 13 or newer;
- full Xcode selected for the process through `DEVELOPER_DIR` or globally with
  `xcode-select`, with macOS SDK, iOS SDK, Swift, and the Metal Toolchain
  component installed. Before any download, the script runs the non-compiling
  `xcrun --sdk macosx metal -v` availability probe; if it fails, install the
  component with `xcodebuild -downloadComponent MetalToolchain`;
- `gettext`/`msgfmt`; and
- the exact CLT macOS 15.2 SDK recorded in the lock file; and
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
extraction. Checksum-verified download archives may remain cached, but every
invocation removes and safely re-extracts both the exact Ghostty source tree and
the official Zig toolchain, and clears Ghostty's exact
install prefix before building. This prevents a prior build's modified toolchain,
generated XCFramework, or resources from satisfying the current build's output
checks. The script uses Ghostty's required checksum-pinned Zig toolchain and
the checksum-pinned archive tool from Xcode 26.5 and sealed CLT macOS 15.2 SDK,
limits the Ghostty build to two jobs, stages a
SwiftPM build against the generated XCFramework, copies Ghostty's resources
and license into the proof app, and applies and verifies an ad-hoc local
signature. It does not launch the app.

Zig's version probe is limited to two minutes and the Ghostty Zig build to one
hour. A timeout terminates the command's process group and fails packaging.
macOS can keep a process in kernel-uninterruptible (`U`) state even after
`SIGKILL`; in that exceptional case the runner reports it and the private build
root must remain untouched until the kernel releases the process.

Only after every proof step succeeds does the script atomically write a
versioned `ghostty-proof.attestation`. It records reviewed inputs and per-build
hashes for diagnosis and consistency checks, but it is not a signature or a
packaging trust root. Production packaging does not accept this standalone
proof directory: it creates a fresh proof below its own mode-700 temporary root,
seals the outputs itself, and consumes a read-only snapshot in the same process
chain. Generated Ghostty outputs are not claimed to be byte-reproducible.

Expected output:

```text
target/ghostty-proof/dist/PrismGhosttyProof.app
target/ghostty-proof/ghostty-proof.attestation
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
