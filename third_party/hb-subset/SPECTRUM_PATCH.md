# Spectrum vendor record

This directory is the exact published source payload of `hb-subset` 0.3.0,
with two small Spectrum-maintained portability patches and rustfmt-only
doc-comment whitespace normalization in `src/lib.rs`.

- Published crate: <https://crates.io/crates/hb-subset/0.3.0>
- Wrapper source repository: <https://github.com/henkkuli/hb-subset-rs>
- crates.io archive SHA-256:
  `c7340b4303dc40254307edbe53fabcdde93b0c3eb0aa85e1fd459beca9863987`
- Wrapper license: MIT; see `LICENSE.md`.
- Bundled engine: HarfBuzz 8.2.2, identified by
  `harfbuzz/src/hb-version.h`.
- Bundled engine source/release: <https://github.com/harfbuzz/harfbuzz/releases/tag/8.2.2>
- Bundled engine license: Old MIT; see `harfbuzz/COPYING`.

Cargo's local registry extraction marker `.cargo-ok` is not part of the
published `.crate` archive and is intentionally not vendored.

## Local patches

The published wrapper's `src/blob.rs` imports
`std::os::unix::prelude::OsStrExt` unconditionally and sends its bytes to
HarfBuzz's narrow C file API. The same Unix-only implementation remains on the
wrapper repository's `main` branch as of 2026-07-21:
<https://github.com/henkkuli/hb-subset-rs/blob/main/src/blob.rs>.

Spectrum replaces only `Blob::from_file`'s path conversion. Rust's
`std::fs::read` opens the native `Path` (including Windows non-UTF-8/UTF-16 path
handling), and `hb_blob_create_or_fail` with `HB_MEMORY_MODE_DUPLICATE` makes an
owned copy before the temporary byte vector is dropped. The public API and
error type are unchanged. Spectrum's subset seam normally uses
`Blob::from_bytes`; this patch also makes the wrapper itself compile and behave
portably when callers use `Blob::from_file`.

The published `build.rs` also passes GCC/Clang's `-std=c++11` spelling to every
C++ compiler. Spectrum selects `/std:c++14` for MSVC (which has no C++11 mode
switch) and retains `-std=c++11` for GNU-like compilers. No HarfBuzz feature,
source, warning, or optimization setting is otherwise changed.

To audit an update, download the named `.crate`, verify the archive checksum,
compare the unpacked payload with this directory excluding this file, the two
patches to `src/blob.rs` and `build.rs`, and the three whitespace-only
doc-comment lines in `src/lib.rs`; then inspect the bundled
`harfbuzz/src/hb-version.h` and both license files again.
