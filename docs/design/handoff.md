# Spectrum handoff

Updated 2026-09-27, after the first demo review. Resume here, then read [workflow preferences](workflow.md),
[direction](../DIRECTION.md), and the repository's [agent guide](../../AGENTS.md).
Apply the project-local `unslop` skill to all writing. Keep replies concise.

## Exact stopping point

The user reviewed the first GPUI demo and approved GPUI and the stock controls.
Their feedback is in [workflow preferences](workflow.md#first-demo-review). The
revision restructures the demo as a mock workspace. The sidebar holds each mode's
controls and the main area shows the work. The user approved the revision's
components and visual style on 2026-09-27 and confirmed the GPUI rebuild.

The user then settled project and import behavior (see workflow preferences).
Projects, membership, unassigned assets, and import batches are in
`spectrum-library` and the CLI (`spectrum projects`, `spectrum imports`,
`images import --project/--new-project`). The demo shows the right-click menu,
All assets and Unassigned views, and "Image" labels.

Deletion follows the user's design: confirmation listing projects, a 30-day
trash (`spectrum delete`, `spectrum trash`), and same-sized canvas placeholders.
Purging removes an image from its import document and deletes the document once
empty. Engine revision caches outside the library are not yet cleaned up.

The user then asked for real imports and a search palette for views. The GPUI
app's Library mode now uses the real library through `spectrum::library::Service`
(the demo depends on the `spectrum` crate until the egui GUI retires): file and
folder import, drag and drop, background thumbnails, the view palette, and the
right-click actions. Adjust and Canvas still use sample data.

The user then asked for Home and project levels. The agreed structure is in
workflow preferences ("Home, projects, and sidebar modes"). The preview now opens
to Home (Projects, Assets), enters projects, shows one sidebar mode at a time,
and edits real images in Color through `Service::adjust`. Multi-select, drag-box
selection, bulk actions, and rename (`lumen_core::Command::RenamePhoto`,
`spectrum rename`) work. Command+1 to 5 and Command+K are handled on the
workspace element; app-level action handlers cannot update the window that is
dispatching the key.

Next, gather feedback on this structure, then move the canvas editor to real
canvases and placeholder replacement on these engine commands, then migrate the editors
incrementally. A full color selector is future work with
undecided timing. Do not restart framework research or start a browser build.

## Build to review

Every push to `main` refreshes a public prerelease tagged `preview`, so the user
can download the signed Mac build without signing in:
<https://github.com/bentsignal/spectrum/releases/download/preview/Spectrum-macos-notarized.zip>.
Artifact links from Actions returned 404 in the user's browser.
Run locally with the command below. The first build's source commit was `775df6e`
([CI run](https://github.com/bentsignal/spectrum/actions/runs/36278142798)).
Display name is Spectrum Preview; bundle ID is `com.bentsignal.spectrum.preview`.
The outer bundle is `Spectrum.app`; keep it apart from the existing app.

## Where the work lives

| Path | Purpose |
| --- | --- |
| `apps/spectrum-demo/` | GPUI development app, isolated from the real library |
| `apps/spectrum-demo/src/theme.rs` | Near-black neutral theme |
| `apps/spectrum-demo/src/workspace.rs` | State, sidebar, title rows, window dragging |
| `apps/spectrum-demo/src/controls.rs` | Dropdown field, segmented control, slider rows |
| `apps/spectrum-demo/src/library.rs` | Library mode and New project dialog |
| `apps/spectrum-demo/src/adjust.rs` | Adjust mode and compare view |
| `apps/spectrum-demo/src/canvas.rs` | Canvas mode, layers, local/global edits |
| `apps/spectrum-demo/src/samples.rs` | Sample assets and placeholder artwork |
| `apps/spectrum-demo/README.md` | Usage |
| `scripts/package-spectrum-demo.sh` | Demo packaging on Mac, Linux, and Windows |
| `packaging/spectrum-demo/Info.plist` | Separate preview bundle identity |
| `tasks/app-managed-library.md` | Ongoing product and engine work |

The demo pins GPUI 0.2.2 and GPUI Component/Assets 0.5.1. Component supplies
common interaction behavior; Spectrum controls the theme. Dropdowns use its popup
menu rather than `Select`, because `Select` items have no gap. Its sliders place
thumbs from the previous frame's bounds, so the workspace renders one extra frame
after a mode change. All state is sample data. Import adds a placeholder photo
and projects exist only in memory. The three modes are a trial layout, not
approved app navigation. Command+1, 2, and 3 are trial bindings.

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
The sidebar holds controls for the current mode; the main area shows the work.
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

The first build passed the full loop, all three platform packages, and Mac
signing and notarization. The revision was screenshot-tested on Linux across all
three modes, dropdowns, slider dragging, the compare view, the New project dialog
with Enter to create, local/global edits, and moving the sidebar.

For automated Linux interaction, use Xvfb with xdotool and ImageMagick. Native
hardware Vulkan could not present into Xvfb; lavapipe worked. Set `DISPLAY=:276`,
unset `WAYLAND_DISPLAY` and `WAYLAND_SOCKET`, and set
`VK_ICD_FILENAMES=/run/opengl-driver/share/vulkan/icd.d/lvp_icd.x86_64.json`.
Run the app inside `nix develop` for `libvulkan`, with Xvfb, xdotool, and
ImageMagick from `nixpkgs` on `PATH`. Kill test processes with `pkill -x`;
`pkill -f spectrum-demo` also matches the calling shell. Direct xdotool control of the KDE desktop triggers a portal
permission prompt; the isolated display avoids it. Test processes were stopped.

The build directory was about 26 GiB. Follow the agent guide's build
space rules and serialize Rust builds. CI omits debug information and incremental
caches to reduce disk use. Timing budgets on shared CI runners are not a current
priority. Retain the working signing credentials and notarization flow.

Commit completed changes on main, push, and verify the remote revision. Run the
full required validation loop after code, packaging, or CI changes. Do not ask the
user to perform checks that can be done on this machine. The next step is feedback on the
Home and project structure.
