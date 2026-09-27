# Spectrum handoff

Updated 2026-09-27. Resume here, then read [workflow preferences](workflow.md),
[direction](../DIRECTION.md), and the repository's [agent guide](../../AGENTS.md).
Apply the project-local `unslop` skill to all writing. Keep replies concise.

## Exact stopping point

The first native GPUI controls demo is built, tested, and delivered for Mac review.
The user has not yet given feedback on its appearance or interactions. Their last
request was to prepare this handoff before moving to a new thread.

Next, review their feedback and revise the demo's components. Do not treat delivery
as visual approval. Once they approve the design, build project creation and asset
import with the shared engine and CLI, then migrate the editors incrementally.
Do not restart framework research or start a browser implementation.

## Build to review

- Application source commit: `775df6e23a6f951a82eb140ee9dcd86670b89715`.
- Verification documentation commit: `f52a904a44613e42f08c26dae125c481fd809dce`.
- [Successful CI run](https://github.com/bentsignal/spectrum/actions/runs/36278142798).
- [Signed Mac download](https://github.com/bentsignal/spectrum/actions/runs/36278142798/artifacts/10918145579).
- Extract the artifact, then extract `Spectrum-macos-notarized.zip` and launch
  `Spectrum.app`. Keep it in a separate folder from the existing Spectrum app.
- Display name is Spectrum Preview; bundle ID is `com.bentsignal.spectrum.preview`.
  The outer bundle is still named `Spectrum.app`. Its only executable is
  `spectrum-demo`. Signing, notarization, stapling, and Gatekeeper assessment passed.
- Local copy: `target/tmp/spectrum-preview-775df6e/`. This is generated output,
  not tracked source. GitHub artifacts may eventually expire; rebuild if needed.

## Where the work lives

| Path | Purpose |
| --- | --- |
| `apps/spectrum-demo/` | New GPUI development app, isolated from the real library |
| `apps/spectrum-demo/src/theme.rs` | Spectrum's neutral gray theme |
| `apps/spectrum-demo/src/gallery.rs` | Demo state, navigation, sidebar, scrolling |
| `apps/spectrum-demo/src/views.rs` | Controls, Adjustments, Assets, and Layers pages |
| `apps/spectrum-demo/src/main.rs` | Window setup and quit actions |
| `apps/spectrum-demo/README.md` | Usage and control coverage |
| `scripts/package-spectrum-demo.sh` | Demo packaging on Mac, Linux, and Windows |
| `packaging/spectrum-demo/Info.plist` | Separate preview bundle identity |
| `.github/workflows/ci.yml` | Packages only the demo; retains full workspace checks |
| `tasks/gpui-controls-demo.md` | Completed implementation and verification record |
| `tasks/app-managed-library.md` | Ongoing product and engine work |

The demo pins GPUI 0.2.2 and GPUI Component/Assets 0.5.1. Component supplies
common interaction behavior; Spectrum controls the theme. It uses sample state
only. Project/import buttons do not create or import real assets. Exposure and
contrast affect a simple gray preview. Scope and layer controls demonstrate UI
state, not completed engine behavior. There is no browser build.

## Decisions to preserve

The user chose native GPUI. Browser access through a local engine daemon was an
optional idea, not a requirement or reason to delay GPUI. The framework research
is historical context, not an open selection process.

The style reference is the ChatGPT desktop app: minimal gray, simple, spacious,
modern, softly rounded, and restrained. The surrounding interface should suit any
creative project. Do not introduce colorful branding or decorative personality.
The reference is not a request to copy chat layout or horizontal toolbars.
The original reference attachment remains on this machine at:

`/home/shawn/.t3/userdata/attachments/fa0938c1-00c7-4d33-87c2-958909e04540-717dd324-e038-4a31-a89c-de8407a0a933.png`

Preserve a full-height content area and a switchable sidebar on either side.
The four demo sections are a component gallery, not approved final app navigation.
The user's shortcut examples and tentative fixed tool area remain in workflow.md.

Spectrum is greenfield. No shipped users or real projects require compatibility
with Lumen/Prism names, file formats, or catalogs. Assets belong to a global library;
projects reference them without copies. Import mixed assets into a current or new
project. Users should not manage scattered project files.

Local editing is the default for an asset used in a canvas. Global editing needs
a deliberate entry and clear dependency impact, including indirect uses. Labels
"Edit locally" and "Edit globally" are preferred. Shared edits propagate; independent
copies remain available. The demo does not implement these engine semantics.

Color correction belongs to compatible visual assets, not only photos. Ordered,
repeatable effects and layers matter. Compositions are assets usable in other
compositions. Future animation should address every editable property. Video,
audio editing, and music follow the current image/canvas workflow and UI work.

## Existing functionality and boundaries

The old working egui desktop app remains in `apps/spectrum`; the user has a Mac
copy for reference. Its performance problems were fixed and the user confirmed
that interaction now feels fast. Do not redo that investigation without evidence.
The user authorized a redesign for GPUI, superseding the original merge's mandate
to preserve the old UI. Keep the old source usable during the transition.

The consolidated `spectrum` CLI, managed library, shared image/canvas references,
and independent copies already exist. Engines still use internal Lumen/Prism names
in `apps/lumen` and `apps/prism`; shared crates live under `crates/`. GUI and CLI
must use the same engine commands. Full creative feature parity is a requirement,
not a claim that every existing control has already been audited. Do not make the
GUI call CLI subprocesses or manually mutate managed library storage.

## Verification and local setup

Use `nix develop` for Rust and native dependencies. Launch the preview with:

```sh
nix develop -c cargo run --release -p spectrum-demo --locked
```

The complete fmt/clippy/workspace-test loop passed locally and in CI. All three
platform packages passed. Linux release testing covered text input, dropdowns,
slider clicks and dragging, color picker opening, layer visibility, scope,
sidebar relocation, resizing, scrolling, and quitting. Section scroll positions
are independent. Mac archive metadata matched the exact source commit.

For automated Linux interaction, use Xvfb with xdotool and ImageMagick. Native
hardware Vulkan could not present into Xvfb; lavapipe worked. Set `DISPLAY=:276`,
unset `WAYLAND_DISPLAY` and `WAYLAND_SOCKET`, and set
`VK_ICD_FILENAMES=/run/opengl-driver/share/vulkan/icd.d/lvp_icd.x86_64.json`.
Obtain tools with `nix shell --inputs-from . nixpkgs#xorg-server nixpkgs#xdotool
nixpkgs#imagemagick`. Direct xdotool control of the KDE desktop triggers a portal
permission prompt; the isolated display avoids it. Test processes were stopped.

The final build directory was about 26 GiB after removing 7 GiB of superseded
debug binaries. Current packages were retained. Follow the agent guide's build
space rules and serialize Rust builds. CI omits debug information and incremental
caches to reduce disk use. Timing budgets on shared CI runners are not a current
priority. Retain the working signing credentials and notarization flow.

Commit completed changes on main, push, and verify the remote revision. Run the
full required validation loop after code, packaging, or CI changes. Do not ask the
user to perform checks that can be done on this machine. The next user input is
visual feedback on the delivered preview.
