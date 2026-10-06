# Spectrum handoff

Updated 2026-10-05, after the one-app overhaul. Resume here, then read
[workflow preferences](workflow.md), [direction](../DIRECTION.md), and the
repository's [agent guide](../../AGENTS.md). Apply the project-local `unslop`
skill to all writing. Keep replies concise.

## Where things stand

Spectrum is one GPUI app (`apps/spectrum-desktop`) over an app-managed library.
Home lists projects; a project shows its images and canvases. Images open in the
image editor (Color with Light, Color, Curves, Mixer, Grading, and Detail
sections; Crop; Info; Compare; Export). Canvases open in the canvas editor
(Overview, Layers, Style, Canvas, and tool modes; the sidebar mode list opens
with ⌘ held for 500 ms). Eight preview reviews shaped the canvas editor; their
record is in [preview feedback](../../tasks/preview-feedback-2026-09-28.md).
The user said the canvas is close to done.

The overhaul made each asset one document in the library, removed the egui apps,
old formats, and the live bridge, and moved the CLI to assets only. Next work is
in [overhaul follow-ups](../../tasks/overhaul-follow-ups.md); the user also wants
a history tree view ([revision lifecycle](../../tasks/revision-lifecycle.md)).

## Build to review

Every push to `main` refreshes a public prerelease tagged `preview`:
<https://github.com/bentsignal/spectrum/releases/download/preview/Spectrum-macos-notarized.zip>.
The bundle is `Spectrum.app`, ID `com.bentsignal.spectrum`, executable
`spectrum-desktop`, with the `spectrum` CLI beside it. Builds are signed and
notarized on main. Give the user this link with each report.

## Decisions to preserve

The user chose native GPUI with GPUI Component 0.5.1; Spectrum controls the
theme. The style reference is the ChatGPT desktop app: minimal gray, spacious,
softly rounded, restrained; no colorful branding. The reference image is at
`/home/shawn/.t3/userdata/attachments/fa0938c1-00c7-4d33-87c2-958909e04540-717dd324-e038-4a31-a89c-de8407a0a933.png`.

Keep a full-height content area and a switchable sidebar on either side holding
the current mode's controls. Photoshop muscle memory matters, values are
typeable, and layouts must not shift. An image is its own asset with color and
crop; layered work happens on a canvas. Placed images default to local editing;
global editing needs a deliberate entry ("Edit locally", "Edit globally").

GPUI notes: Component sliders place thumbs from the previous frame's bounds, so
the workspace renders an extra frame after a mode change. Component emits
`InputEvent::Change` even from `set_value`; handlers must compare values.
Dropdowns use its popup menu rather than `Select`.

## Verification and local setup

Use `nix develop` for Rust and native dependencies. Run the app with
`nix develop -c cargo run --release -p spectrum-desktop --locked`, against a test
library via `SPECTRUM_LIBRARY`; never the user's real library.

For automated Linux interaction, use Xvfb with xdotool and ImageMagick from
`nixpkgs`. Hardware Vulkan cannot present into Xvfb; lavapipe works: unset
`WAYLAND_DISPLAY` and `WAYLAND_SOCKET`, set `DISPLAY` to a private display and
`VK_ICD_FILENAMES=/run/opengl-driver/share/vulkan/icd.d/lvp_icd.x86_64.json`,
and run with `GPUI_X11_SCALE_FACTOR=2`. Kill test processes with a bracketed
`pkill -f` pattern such as `release/spectrum-deskto[p]`: `pkill -x` cannot match
names over 15 characters, and an unbracketed pattern matches the calling shell.
Stray app processes skew benchmarks. Direct xdotool control of the KDE desktop
triggers a portal permission prompt; the isolated display avoids it.

Commit completed changes on main, push, and verify the remote revision. Run the
full validation loop after code, packaging, or CI changes, and the strict
benchmarks after rendering or interaction changes. Do not ask the user to
perform checks that can be done on this machine.
