---
status: done
priority: normal
---

# Reproducible NixOS development

Provide a locked project-local Rust and native dependency environment. Preserve
all existing Linux, Windows, and macOS GitHub Actions builds and runner choices.

Acceptance: format, workspace Clippy and tests pass; Lumen and Prism release
binaries and Linux packages build; both GUIs open in the available desktop;
document daily commands and any environmental limitations.

Validated on x86_64 NixOS with pinned NixOS 26.05 / Rust 1.95. The shell supplies
bindgen/libclang, native GUI libraries, and workspace-local temporary fixtures.
A behavior-equivalent Lumen boolean simplification resolves a Clippy warning.
Format and Clippy passed; workspace tests passed (1,055 passed, five ignored).
The release workspace build, both Linux packaging scripts, and both packaged
CLI schema commands passed. Both packaged GUIs visibly opened through XWayland.
Native Wayland rendered but KDE marked both windows unresponsive; the documented
per-process X11 fallback works. No native Wayland fix is included in this setup.
The aarch64 shell evaluates but was not built on this x86_64 machine.

GitHub Actions, runner types, billing, and system configuration were unchanged.
Passwordless sudo is unavailable in the agent session (`no new privileges`);
project-local setup did not require it. Local packages require the Nix store
and development-shell runtime paths. See [Development](../docs/DEVELOPMENT.md).

Build output peaked at 13 GiB. Removed 3 GiB of disposable debug incremental
compiler caches after validation; 9.2 GiB remain, including current binaries,
packages, and smoke test artifacts under `target/nixos-validation`.
