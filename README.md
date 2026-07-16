# Lumen

Lumen is a small, native, nondestructive photo editor written in Rust. It gives
you a focused place to import a shoot, move quickly between photos, make the
essential tonal and color edits, copy a look across images, and export finished
files.

The architecture is CLI-first. The desktop UI calls the same typed command
engine as the `lumen` CLI, so people, scripts, and agents all get the same
capabilities.

## What works today

- Drag/drop or file-picker import
- Horizontal filmstrip and arrow-key navigation
- Live preview for exposure, white balance, contrast, highlights, shadows,
  whites, blacks, clarity, vibrance, saturation, and vignette
- Rotation and horizontal/vertical flips
- Nondestructive `.lumencatalog` sidecars; source photos are never changed
- Undo/redo with `Ctrl+Z` / `Cmd+Z`
- Copy, paste, and reset edits
- Full-resolution JPEG, PNG, TIFF, and WebP export
- JSON-speaking CLI with a raw command protocol for agents
- Native builds for Windows, macOS, and Linux

## Run it

Install the stable [Rust toolchain](https://rustup.rs), clone this repository,
and launch the native editor:

```sh
cargo run --release --bin lumen-gui
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
cargo run --release --bin lumen -- --catalog shoot.lumencatalog import photos/*.jpg

# Inspect IDs, edit photo 1, copy its look, and export it
cargo run --release --bin lumen -- --catalog shoot.lumencatalog list
cargo run --release --bin lumen -- --catalog shoot.lumencatalog edit 1 \
  --exposure 0.6 --temperature 12 --shadows 20 --vibrance 8
cargo run --release --bin lumen -- --catalog shoot.lumencatalog copy-edits \
  --from 1 --to 2 3 4
cargo run --release --bin lumen -- --catalog shoot.lumencatalog export \
  1 finished.jpg --quality 92

# Discover the machine-facing protocol
cargo run --release --bin lumen -- schema
```

See [CLI.md](docs/CLI.md) for the full surface and
[ARCHITECTURE.md](docs/ARCHITECTURE.md) for the design.

## Supported images

JPEG, PNG, TIFF, and WebP are supported today. Camera RAW support is deliberately
planned as a follow-up: it needs a high-quality, pure-Rust decoding and color
management path instead of a large native dependency.

## Scope of “100% Rust”

Lumen's application, command engine, catalog, UI, and pixel pipeline are Rust.
Like any native desktop program, it calls the operating system's windowing and
file-dialog APIs through Rust crates. It does not ship an Electron/web runtime,
C++ image engine, database server, or background service.

## License

MIT

