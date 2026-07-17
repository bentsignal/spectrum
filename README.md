# Lumen creative suite

This repository is the Rust workspace for Lumen, a focused photo-development
library, and Mica, a focused layered image editor. They are separate native apps
with shared imaging primitives and the same CLI-first command architecture, so
each interface stays clean while assets can move between them.

- **Lumen** develops, organizes, culls, and batch-exports photos.
- **Mica** creates layered compositions with raster content, text, transforms,
  masks, clipping, and nondestructive adjustments.

See [Suite direction](docs/SUITE.md) for the shared architecture and application
handoffs, or [Mica](docs/MICA.md) for the layered editor.

## Lumen

Lumen is a small, native, nondestructive photo editor written in Rust. It gives
you a focused place to import a shoot, move quickly between photos, make the
essential tonal and color edits, copy a look across images, and export finished
files.

The architecture is CLI-first. The desktop UI calls the same typed command
engine as the `lumen` CLI, so people, scripts, and agents all get the same
capabilities.

## What works today

- Parallel drag/drop or file-picker import with deterministic performance budgets
- Chronological Library timeline with lightweight shoot batches inside one catalog
- Vertical filmstrip, arrow-key navigation, multi-selection, and keep/reject culling filters
- Pure-Rust Sony ARW metadata, embedded-preview decoding, and full-resolution development
- Zoom, pan, direct on-image crop handles, straighten, rotation, and flips
- Live basic, presence, detail, and vignette controls
- Eight-color HSL mixer and point-editable master/red/green/blue tone curves
- Shadows, midtones, and highlights color grading
- Original/edited side-by-side comparison, RGB histogram, and camera/lens details
- Nondestructive repair brush for dust and small blemishes
- Rotation and horizontal/vertical flips
- Portable `.lumencatalog` libraries with relative iCloud-friendly source references
- Nondestructive edits and removal; source photos are never changed or deleted
- Persistent per-photo edit history with `Ctrl+Z` / `Cmd+Z` navigation
- Copy/paste edits, reusable named presets, and confirmed history-preserving reset
- Configurable single or batch JPEG, PNG, TIFF, and WebP export with size estimates
- JSON-speaking CLI with a raw command protocol for agents
- Native builds for Windows, macOS, and Linux

## Run Lumen

Install the stable [Rust toolchain](https://rustup.rs), clone this repository,
and launch the native editor:

```sh
cargo run --release --bin lumen-gui
```

Open a catalog directly by passing its path after `--`:

```sh
cargo run --release --bin lumen-gui -- path/to/library.lumencatalog
```

The first release build takes a few minutes because it compiles the native UI
stack. Later builds are incremental. For smaller, optimized distributable
binaries, always use `--release`.

On Windows, the result is `target\release\lumen-gui.exe`; on macOS and Linux it
is `target/release/lumen-gui`.

## CLI quick start

All successful CLI output is JSON on stdout; failures are JSON on stderr with a
nonzero exit code.

```sh
# Create a catalog and import a shoot
cargo run --release --bin lumen -- --catalog shoot.lumencatalog init "Friday shoot"
cargo run --release --bin lumen -- --catalog shoot.lumencatalog import photos/*.{ARW,jpg}
cargo run --release --bin lumen -- --catalog shoot.lumencatalog batch-rename 1 "Friday portraits"

# Inspect IDs, edit photo 1, copy its look, and export it
cargo run --release --bin lumen -- --catalog shoot.lumencatalog list
cargo run --release --bin lumen -- --catalog shoot.lumencatalog edit 1 \
  --exposure 0.6 --temperature 12 --shadows 20 --vibrance 8
cargo run --release --bin lumen -- --catalog shoot.lumencatalog copy-edits \
  --from 1 --to 2 3 4
cargo run --release --bin lumen -- --catalog shoot.lumencatalog export \
  1 finished.jpg --quality 92

# Save a reusable look, apply it without replacing crop/rotation, and batch export
cargo run --release --bin lumen -- --catalog shoot.lumencatalog preset-save \
  "Warm portrait" --from 1
cargo run --release --bin lumen -- --catalog shoot.lumencatalog preset-apply 1 2 3 4
cargo run --release --bin lumen -- --catalog shoot.lumencatalog export-batch \
  1 2 3 4 --directory finished --format jpeg --quality 92

# Discover the machine-facing protocol
cargo run --release --bin lumen -- schema

# Measure imports, tone-curve responsiveness, and full 24 MP export throughput
cargo run --release --bin lumen -- benchmark
```

See [CLI.md](docs/CLI.md) for the full surface and
[ARCHITECTURE.md](docs/ARCHITECTURE.md) for the design.

## Supported images

JPEG, PNG, TIFF, WebP, and Sony ARW are supported. ARW files are demosaiced,
white-balanced, color-calibrated, and converted to sRGB by the pure-Rust
`rawler` pipeline. Imports inspect RAW metadata without a full demosaic and UI
previews use embedded camera previews when available. Originals remain immutable.

## Packages

Build an optimized Lumen package from the matching operating system:

```sh
bash scripts/package-macos.sh
bash scripts/package-linux.sh
pwsh scripts/package-windows.ps1
```

GitHub Actions builds and publishes artifacts for Windows, macOS, and Linux on
every push to `main`.

Mica has matching app-specific scripts that build only Cargo package `mica`:

```sh
bash scripts/package-mica-macos.sh
bash scripts/package-mica-linux.sh
pwsh scripts/package-mica-windows.ps1
```

## Scope of “100% Rust”

Lumen's application, command engine, catalog, UI, and pixel pipeline are Rust.
Like any native desktop program, it calls the operating system's windowing and
file-dialog APIs through Rust crates. It does not ship an Electron/web runtime,
C++ image engine, database server, or background service.

## License

MIT

See [THIRD_PARTY.md](THIRD_PARTY.md) for dependency notices, including the
LGPL-2.1 `rawler` RAW decoder.
