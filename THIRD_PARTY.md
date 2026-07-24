# Third-party notices

The Spectrum creative suite is MIT licensed. Its dependency graph is recorded
exactly in `Cargo.lock`.

Sony ARW decoding and development uses `rawler` 0.7.2, Copyright (c) the
dnglab/rawler contributors, under the GNU Lesser General Public License v2.1.
Source and license text: <https://github.com/dnglab/dnglab/tree/main/rawler>.

Prism uses the Ubuntu Light font designed by Dalton Maag and distributed by
`epaint_default_fonts` for portable text-layer rendering. Ubuntu Font Family is
licensed under the Ubuntu Font Licence 1.0:
<https://ubuntu.com/legal/font-licence>.

Spectrum's disconnected font-subset candidate vendors `hb-subset` 0.3.0,
Copyright (c) 2023 Henrik Lievonen, under the MIT License. The vendor record,
published archive checksum, and Spectrum's two audited portability patches are in
`third_party/hb-subset/SPECTRUM_PATCH.md`; the license text is in
`third_party/hb-subset/LICENSE.md`.

That wrapper bundles HarfBuzz 8.2.2, Copyright its contributors, under the Old
MIT License. The complete notice is preserved in
`third_party/hb-subset/harfbuzz/COPYING`. The candidate is not connected to a
Spectrum application until its supported envelope passes the required
cross-platform conformance corpus.

The candidate is path-pinned to the reviewed vendored `hb-subset` package in
`Cargo.lock`; the approved bounded build and licensed corpus validation have
completed without connecting the candidate to a Spectrum application.

The optional terminal proof harness and explicitly Ghostty-enabled macOS
package builds statically link Ghostty 1.3.1, Copyright (c) 2024 Mitchell
Hashimoto, under the MIT License. The exact official source archive checksum,
annotated tag object, peeled source revision, toolchain, and generated artifact
contract are recorded in
`packaging/spectrum-terminal/macos/ghostty-proof.lock`. Both explicitly
Ghostty-enabled app bundles include Ghostty's license as
`Contents/Resources/GHOSTTY-LICENSE`. Ordinary Lumen and Prism packages do not
include or load Ghostty. Compatible hosts use the checksummed official Zig
0.15.2 archives; affected arm64 SDKs require an already-installed Homebrew
`zig@0.15` bottle carrying Homebrew's SDK patch.

Packaged builds include this notice. Anyone distributing the suite should review
the LGPL requirements for their distribution model and retain a relinkable or
otherwise compliant form of the application.
