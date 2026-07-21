# Conformance fixture provenance

`noto-sans-static-source.ttf` is a derived test fixture from Noto Sans Regular at
notofonts/noto-fonts commit `ffebf8c1ee449e544955a7e813c54f9b73848eac`:

<https://github.com/notofonts/noto-fonts/blob/ffebf8c1ee449e544955a7e813c54f9b73848eac/hinted/ttf/NotoSans/NotoSans-Regular.ttf>

The upstream input SHA-256 is
`b85c38ecea8a7cfb39c24e395a4007474fa5a4fc864f6ee33309eb4948d232d5`.
The accepted static fixture retains `A` through `Z`, drops hinting, and drops
tables outside its locked basic-static generation profile using bundled
HarfBuzz Subset 8.2.2 through `hb-subset = 0.3.0`. Its SHA-256 is
`1de794fb16bb4fc99afef5d597b909791230be4ea216c5762797f09d9d56a04c`.

`noto-sans-rich-rejected.ttf` is the exact hinted upstream input. Its SHA-256 is
`b85c38ecea8a7cfb39c24e395a4007474fa5a4fc864f6ee33309eb4948d232d5`.
It proves that STAT and, after STAT is removed, TrueType hinting fail closed.
`noto-sans-layout-source.ttf` is the pinned unhinted derived fixture for accepted
layout coverage. It retains `AV`, `ffi`, and U+00C5, exercising real GPOS
positioning, GSUB ligature closure, GDEF, and a composite glyph without making
a hinted-interpreter claim. Its SHA-256 is
`05549e889a11eb65b542a071e97d71f4333caf5a88408ab10a1ac3de25d4be3a`.

The integration test derives a third case in memory from the locked static
fixture. It appends a checksummed cmap format 14 subtable containing both a
default mapping (`A` plus U+FE0F) and a non-default mapping (`B` plus U+FE0F to
the `C` glyph). This keeps the real Noto outlines, names, and metrics while
making the exact UVS table bytes reviewable in `tests/support.rs`. Its derived
source hash is locked after the approved unhinted static fixture is generated.
The test also records the current fontdue 0.9.3 limitation: indexed glyphs are
materialized from nominal cmap and GSUB data, not format-14-only alternates.
Spectrum therefore requires exact HarfBuzz mapping and ttf-parser
outline/metric parity for that alternate but does not claim Prism renders it.

Run `./generate.sh` from any directory to download the exact revision, verify
the source hash in the pinned Rust generator, and reproduce the checked font
files. `fixtures.lock` records the source revision, URL, hashes, tool version,
flags, table policy, and command. The test suite independently compares the
checked files with those golden hashes.

`output-goldens.lock` fixes reviewed SHA-256 assertions for the produced A/B,
default/non-default UVS, and rich-layout subsets. Each value came from the
approved pinned-engine run and was locked before the passing rerun. No value is
inferred from a different HarfBuzz build.

The present proof is deliberately not a broad production corpus. Real variable
TrueType and CFF/OTF rejection fixtures are pinned at exact upstream revisions:
`noto-sans-variable-rejected.ttf` comes from Google Fonts and
`noto-sans-cff-rejected.otf` comes from notofonts/noto-fonts. Their exact source
and license URLs and hashes are recorded in `fixtures.lock`; the matching
license texts are checked in beside them. Until the full licensed corpus and
the same test matrix pass on macOS, Linux, and Windows, this crate remains
disconnected from Prism export.

Noto is licensed under the SIL Open Font License 1.1; see `OFL.txt`,
`OFL-google-fonts.txt`, and `LICENSE-notofonts.txt`.
