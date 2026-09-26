# Spectrum CLI

One executable controls the shared library and both editors: `spectrum`.
Successful commands print JSON to stdout; operational errors print JSON to stderr
and exit nonzero. Argument errors and `--help` use clap's normal text output.
There are no separate Lumen or Prism executables.

## Library workflow

`--library <directory>` or `SPECTRUM_LIBRARY` selects a library; otherwise Spectrum
uses its platform application-data location. Documents and originals are managed
inside that directory. Help, schemas, and benchmarks do not initialize a library.

```sh
spectrum library
spectrum images import /path/to/image.jpg
spectrum images list
spectrum canvas new "Composition" --width 1920 --height 1080
spectrum canvas list
spectrum canvas place <canvas-uuid> <image-uuid>
spectrum images adjust <image-uuid> '{"exposure":0.4,"vibrance":12}'
spectrum canvas export <canvas-uuid> /path/to/output.png
spectrum images export <image-uuid> /path/to/output.jpg --quality 90 --max-size 3200
spectrum copy <asset-uuid>
```

Placement creates a live reference. Image edits propagate to referencing canvases;
canvas export resolves current image edits. Copy creates independent content,
recursively copying a canvas's image references. Exports must be outside the
managed library and cannot overwrite imported originals.

`images command <asset-uuid> <json>` and `canvas command <asset-uuid> <json>`
execute core commands. Library mutations use the authenticated desktop bridge
when the document is open; otherwise they commit directly. `schema` describes
both engines; `images schema` and `canvas schema` describe individual protocols.

## Complete editor command surface

Advanced commands target `--asset <uuid>` or `--document <internal-path>`.
The asset selector resolves the document; it does not rewrite IDs inside the
command. Image item IDs and canvas layer IDs are **document-local integers**.
Library listings include an image's `item`; `inspect` returns full editor state.
No default document is created in the working directory. Advanced `init` creates
an isolated engine document at an explicit path; normal creation uses the library.

```sh
spectrum images --asset <image-uuid> inspect
spectrum images --asset <image-uuid> edit <item> --exposure -0.35 --highlights -28
spectrum images --asset <image-uuid> crop <item> --x 0.05 --y 0.05 --width 0.9 --height 0.85
spectrum images --asset <image-uuid> curve <item> master --points '0,0;0.4,0.55;1,1'
spectrum images --asset <image-uuid> history <item>
spectrum canvas --asset <canvas-uuid> inspect
spectrum canvas --asset <canvas-uuid> add-text "Hello" --x 100 --y 100
spectrum canvas --asset <canvas-uuid> run '{"command":"undo"}'
```

Image commands include get, edit, crop, HSL, curves, grading, spot repair, pick,
batch rename, history navigation, reset, presets, copying edits, rotate, flip,
remove, batch export, raw command batches, collaboration, and live inspection.
Canvas commands include text, images, shapes, paths, painting, selection, masks,
effects, transforms, alignment, guides, typography, font inspection/subsetting,
layer transfer, history, raw command batches, collaboration, and live inspection.
Run `spectrum images --help`, `spectrum canvas --help`, or a command's `--help`
for complete flags. Creation/import/export use the library commands above;
`inspect` replaces the old document-level `list`. The old `from-lumen` conversion
is replaced by linked `canvas place`.

## Embedded terminals and collaboration

Terminals put the bundled `spectrum` executable on PATH and supply
`SPECTRUM_IMAGES_DOCUMENT` or `SPECTRUM_CANVAS_DOCUMENT`, plus `SPECTRUM_SESSION`,
`SPECTRUM_LIVE_MODE`, and `SPECTRUM_LIVE_BINDING_ID`. Each domain uses only its own
document variable. Document paths are passed as environment data, never shell
source. Explicit target flags override the terminal's document context.

Advanced editor commands retain explicit session semantics. Start an agent
session, then use its returned session ID for subsequent commands. Embedded
terminals require the authenticated live bridge; never bypass it to edit an open
document. Outside the app, use `--live required` when editing a running workspace.
Some image convenience commands require raw `live apply` in live mode; its help
and schema describe the complete protocol. Unsupported live actions fail rather
than silently writing directly.

```sh
spectrum images --document <path> --live off agent start <item> --mode together
spectrum images --document <path> --session <agent-session> --live required edit <item> --exposure 0.5
spectrum canvas --document <path> agent start --mode separate
spectrum canvas --document <path> --session <agent-session> --live required add-text "Hello"
```

`together` follows agent revisions until a competing human edit; `separate`
keeps the human cursor independent. Originals remain immutable. Completed
semantic edits publish durable revisions; raw arrays commit as atomic batches.

## Benchmarks

```sh
spectrum images benchmark --strict
spectrum canvas benchmark --strict
```

Both accept `--profile hosted-ci` for shared-runner budgets. Image benchmarking
also accepts `--raw-import <path>`. Reports distinguish targets from regression
budgets; `--strict` returns nonzero when a budget is missed. Source generation
and warm-up are excluded. Benchmark definitions live with the consolidated CLI.
