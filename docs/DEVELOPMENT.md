# NixOS development

From the repository root, enter the locked Linux development environment:

```sh
nix develop
```

If flakes are not enabled in your Nix configuration, use
`nix --extra-experimental-features 'nix-command flakes' develop`.
The first entry downloads dependencies. `flake.lock` pins NixOS 26.05 packages
and Rust 1.95, including Cargo, Clippy, rustfmt, rust-analyzer, the C/C++ compiler,
CMake, pkg-config, libclang/bindgen support, and native Wayland/X11/OpenGL
libraries. No system Rust install or sudo is needed. Other platforms continue to use `rust-toolchain.toml` and
the existing GitHub Actions build matrix.

Run these commands inside the shell:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --all-targets --locked
cargo build --release --workspace --bins --locked
bash scripts/package-spectrum-linux.sh
./target/release/spectrum images schema
./target/release/spectrum canvas schema
./target/release/spectrum-gui
```

For a single command without an interactive shell, use, for example,
`nix develop -c cargo test --workspace --all-targets --locked`.
Start an editor from this shell to inherit the toolchain and library paths.
Keep Cargo builds serialized and monitor `du -sh target` per the agent guide.
The shell sets `TMPDIR` to `target/tmp` so test fixtures stay on the workspace
mount. This avoids revision-storage inode exchange failures when `/tmp` is a
separate bind mount from the workspace and the desktop application cache.

Launch GUIs from a terminal in your graphical session so they inherit the
Wayland/X11 and desktop portal environment. The shell exposes dynamically
loaded native libraries and prefers `/run/opengl-driver/lib` for the running
NixOS generation's graphics driver.

On the tested KDE Wayland session, native Wayland windows rendered but KDE
marked them unresponsive. X11/XWayland opened the packaged GUI normally.
Use this per-process fallback on that session (requires XWayland and `DISPLAY`):

```sh
env -u WAYLAND_DISPLAY -u WAYLAND_SOCKET ./target/release/spectrum-gui
```

The Linux script produces `target/dist/spectrum-linux`. Run its binaries inside
`nix develop` too. Packages
built on NixOS reference the Nix store; these directories are local development
artifacts, not portable distributions for other Linux machines. Use the
existing Linux CI artifacts for the existing distribution workflow.

To deliberately update the pinned environment, run `nix flake update`, then
repeat all checks, builds, packaging, and GUI smoke tests before committing
the changed lock file. The development shell supports x86_64 and aarch64 Linux;
validate on the target architecture before relying on a new one.
