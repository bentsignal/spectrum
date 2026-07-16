# Lumen

Lumen is a small, cross-platform, nondestructive photo editor written in Rust.
It is designed CLI-first: the native GUI is an interface over the same command
engine that powers the `lumen` command-line tool, so people and agents have the
same capabilities.

> Early development: the first playable release is being built now.

## Goals

- Native desktop app for Windows, macOS, and Linux
- 100% Rust application and image-processing pipeline
- Nondestructive catalog files; original photos are never modified
- Fast keyboard and mouse workflow for editing many photos
- Complete, structured CLI coverage for automation and agents
- Small release builds and minimal background work

## Build prerequisites

Install the stable [Rust toolchain](https://rustup.rs), then run:

```sh
cargo run --release --bin lumen-gui
cargo run --release --bin lumen -- --help
```

## Supported images

The first release supports JPEG, PNG, TIFF, and WebP. Camera RAW support is a
planned follow-up because it deserves a deliberate pure-Rust decoding pipeline.

## License

MIT

