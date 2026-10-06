# Spectrum

Spectrum is one native creative app: an app-managed library of images and
canvases, an image editor for color and crop, and a canvas editor for layered
composition. Video, audio, and music come next.

Run the desktop app with `cargo run --release -p spectrum-desktop`. The
`spectrum` CLI (`cargo run --release -p spectrum --bin spectrum -- --help`)
exposes every library and editing operation as JSON.

See [Architecture](docs/ARCHITECTURE.md), [CLI](docs/CLI.md), and
[Development](docs/DEVELOPMENT.md).
