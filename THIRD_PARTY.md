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

Spectrum's font subsetting and Prism shaped-text layout vendor `hb-subset` 0.3.0,
Copyright (c) 2023 Henrik Lievonen, under the MIT License. The vendor record,
published archive checksum, and Spectrum's two audited portability patches are in
`third_party/hb-subset/SPECTRUM_PATCH.md`; the license text is in
`third_party/hb-subset/LICENSE.md`.

That wrapper bundles HarfBuzz 8.2.2, Copyright its contributors, under the Old
MIT License. The complete notice is preserved in
`third_party/hb-subset/harfbuzz/COPYING`. Prism's permanently versioned
`HarfBuzzV1` policy uses this exact in-process build for bounded OpenType
shaping; it never discovers a runtime library or invokes an external process.

The wrapper is path-pinned to the reviewed vendored `hb-subset` package in
`Cargo.lock`; its approved bounded build and licensed corpus validation cover
both subsetting and the public one-font-run shaping seam.

Prism `HarfBuzzV1` also pins `unicode-bidi` 0.3.18, `unicode-linebreak` 0.1.5,
`unicode-script` 0.5.8, and `unicode-segmentation` 1.13.3 under their published
MIT/Apache-2.0-compatible terms. Those releases carry the frozen UAX #9, #14,
#24, and #29 data used by this layout version. BCP-47 parsing and alias
canonicalization use ICU4X `icu_locale` and `icu_locale_core` 2.2.0 under the
Unicode-3.0 license. Indexed glyph outlines are read with `ttf-parser` 0.21.1
and rasterized with `tiny-skia` 0.11.4 under MIT/Apache-2.0-compatible terms.
Exact resolved dependency records are retained in `Cargo.lock`.

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
