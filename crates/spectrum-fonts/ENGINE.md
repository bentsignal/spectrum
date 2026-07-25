# In-process shaping and font subset engine

Spectrum's shaping and subset substrate is HarfBuzz 8.2.2, bundled by the exact
published `hb-subset = 0.3.0` source and linked in process through its `bundled`
feature. The published wrapper is vendored with two audited portability patches;
see `../../third_party/hb-subset/SPECTRUM_PATCH.md`. This
path does not invoke a font tool executable or load a system HarfBuzz library
at runtime. Its C++ compilation and bindgen requirements have not yet been
proven on Spectrum's macOS, Linux, and Windows CI targets.

## Run-level shaping contract

`HarfBuzzShaper` is the public production-safe seam for one already-resolved
font run. It borrows immutable caller-owned bytes, validates the selected face
with both `ttf-parser` and HarfBuzz, scales positions to the face units per em,
and returns glyph IDs, UTF-8 byte clusters, HarfBuzz glyph flags, advances,
offsets, and resolved run properties. Direction and ISO 15924 script may be
explicit or guessed. Language may be explicit; omission deterministically uses
`und` instead of consulting the process locale.

HarfBuzz default features remain enabled. Callers may provide at most 32
ordered feature overrides with lowercase four-byte OpenType tags. Overrides
may cover the full run or a valid UTF-8 byte range. One call accepts at most
64 KiB of UTF-8, 16,384 Unicode scalars, and 65,536 returned glyphs. Font data
retains the shared 64 MiB limit. Native faces, fonts, and buffers are all
RAII-owned, allocation results are checked, and native array lengths, glyph
IDs, flag bits, and byte clusters are validated before safe Rust slices or
results are exposed.

This seam shapes a run; it does not claim to implement Unicode bidi paragraph
resolution, font fallback, UAX #14 line breaking, caret or selection mapping,
IME composition, or Prism renderer integration. Those remain explicit later
layers. The subsetting conformance path adapts its codepoint samples through
this same shaper rather than maintaining a second HarfBuzz FFI implementation.

## Subset candidate

The choice is intentionally conservative:

- HarfBuzz documents `glyf`, CFF/CFF2, OpenType variable outlines, color outline
  and bitmap tables, and OpenType GSUB/GPOS/GDEF layout closure. Spectrum does
  not treat documented engine coverage as application conformance.
- `allsorts` is mature and pure Rust, but its subset output is described as
  suitable for PDF embedding and does not provide the editable-font layout
  closure and variable-font preservation required here.
- `font-subset` documents that it drops positioning/kerning data.
- `oxifont-subset` advertises broader pure-Rust coverage, but its implementation
  and release history are too new to use as a production prerequisite without
  a substantially larger independent corpus.

Primary references:

- <https://github.com/harfbuzz/harfbuzz>
- <https://harfbuzz.github.io/harfbuzz-hb-subset.html>
- <https://github.com/harfbuzz/harfbuzz/blob/main/BUILD.md>
- <https://github.com/henkkuli/hb-subset-rs>
- <https://github.com/yeslogic/allsorts>

The current seam accepts only standalone, static, unhinted TrueType `glyf` fonts
with a strictly enumerated core table set. Global TrueType hint tables and any
local `glyf` instruction stream fail closed: the candidate does not claim hinted
interpreter parity. The envelope includes OpenType GSUB/GPOS/GDEF tables. A
request for a layout font must
provide exact shaping samples; each sample is closed by HarfBuzz and shaped
before and after subsetting with default features, character-level clusters,
fixed `und` language, and guessed direction/script. Glyphs are compared through
the subset plan's old-to-new map, including cluster, glyph flags, advances, and
offsets. Every shaped closure glyph also receives outline, horizontal-metric,
and indexed-raster comparison, except a mapped non-default UVS alternate that
is reachable only through cmap format 14. `fontdue 0.9.3` does not materialize
such glyphs in its indexed cache; HarfBuzz mapping and exact ttf-parser
outline/metric parity remain mandatory for that narrow case.

The seam still rejects TTC, CFF/CFF2, STAT, BASE, MATH, VORG, `kern`, TrueType
hint programs and local instructions, every
variable-font table including VARC, all color/bitmap/SVG tables including
`sbix`, Graphite/AAT, and unknown tables. Subset closure validation continues
to use HarfBuzz defaults, guessed direction/script, and deterministic `und`.
The public run shaper supports bounded feature overrides and explicit run
properties, but those inputs are not yet part of `SubsetRequest` and therefore
are not claimed by the subset candidate.

Prism's optimized-copy transaction is the only production caller allowed to
persist a candidate artifact. It additionally verifies immutable source identity,
linear history, reachable assets, and exact full/region render parity before
atomic publication; this does not authorize the general export path. The
path-pinned dependency and licensed conformance corpus must continue to pass on
macOS, Windows, and Linux. The approved local bounded build and corpus pass are
complete; the platform build matrix explicitly runs
`cargo test -p spectrum-fonts --locked` so the library-only crate and its exact
output goldens cannot be skipped by `--workspace --bins`. The accepted corpus
uses pinned unhinted static and layout fixtures; the
exact hinted upstream font is a rejection case until Spectrum has portable
TrueType interpreter/validator evidence across multiple ppem values. A derived
cmap format 14 case covers default and non-default mappings, including actual
base-plus-selector shaping. Real variable and CFF/OTF sources remain
fail-closed cases pending the approved download. Runtime guards must
independently require byte reduction, raw name-record and fsType retention,
parser reload, every requested nominal cmap mapping, every requested UVS
mapping's presence and default/non-default kind, resolved UVS outline and
horizontal-metric parity, default-feature shaping parity for every supplied
sample, shaped-closure outline/metric/raster parity, and app-renderer parity for
nominal codepoints. `fontdue` is Spectrum's current unhinted renderer; these
checks do not establish TrueType bytecode behavior. Nominal outlines and
advances are compared for every requested scalar. The Rust boundary enforces
explicit source, table, name-copy, glyph, request, sample-count, and shaping-
length limits before invoking bundled native code. HarfBuzz 8.2.2 grows a
shaping buffer to `max(input_length * 64, 16,384)`; Spectrum therefore caps
each sample at 256 scalars and rejects any returned glyph count above 16,384
before constructing native-result slices. At most 256 samples and 16,384 total
sample scalars are accepted per request.
The runtime guard does not inventory or reject additional unrequested cmap/UVS
mappings, and Prism's current fontdue renderer does not render UVS alternates.
Those remain corpus prerequisites. The caller owns immutable source
retention, provenance, and legal license decisions. OS/2 fsType is only
technical embedding metadata. Any failed prerequisite leaves the original full
embedded font in use.
