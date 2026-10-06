# Spectrum CLI

One executable, `spectrum`, controls the library and every editor. Successful
commands print JSON to stdout; failures print `{"ok": false, "error": ...}` to
stderr and exit nonzero. `--help` works at every level. Help, schemas, and
benchmarks do not create a library.

`--library <directory>` or `SPECTRUM_LIBRARY` selects a library; otherwise the
default library in Spectrum's application-data directory is used. In packages,
`spectrum` is the desktop executable started under that name.

## Agents and history

Every asset's history is a tree of revisions; the person and each agent have
their own place in it, and nothing is overwritten. By default the CLI works
**together** with the person: it starts from the revision the person is on,
and the person (and the open asset in the app) follows its edits until they
edit something themselves. Their edit branches, and the CLI's next command
starts again from it. Before starting, ask the person whether to work together
or separately.

```sh
spectrum agent start <asset> --mode separate --name "Claude"   # prints a session
spectrum images --asset <image> --session <session> edit --exposure 0.5
spectrum agent status <asset> --session <session>
spectrum canvas --asset <canvas> history          # revisions and every session
spectrum canvas --asset <canvas> history-jump <revision>
```

A separate session never moves the person. `SPECTRUM_SESSION` can stand in for
`--session`. `history-jump` moves only the session running the command.

## Library commands

These take asset UUIDs as arguments.

```sh
spectrum library [--unassigned]
spectrum images import <files...> [--project <uuid> | --new-project "Trip"]
spectrum imports
spectrum projects list|create|show|rename|delete|add|remove
spectrum rename <asset> "New name"
spectrum copy <asset>           # a canvas copy also copies its images
spectrum delete <asset>         # to the trash for 30 days
spectrum trash list|restore <asset>|empty
spectrum images list
spectrum images adjust <image> '{"exposure":0.4,"vibrance":12}'
spectrum images apply-edits <from-image> <image>...   # crop included
spectrum images command <image> '{"action":"reset"}'
spectrum images export <image> out.jpg [--quality 90] [--max-size 3200]
spectrum canvas list
spectrum canvas new "Poster" [--width 1920 --height 1080 --background 18191dff --project <uuid>]
spectrum canvas place <canvas> <image>
spectrum canvas command <canvas> '[{"command":"add_text","text":"Hi","name":null,"font_size":48,"color":[255,255,255,255],"x":0,"y":0}]'
spectrum canvas export <canvas> out.png [--max-size 2048]
spectrum schema
```

A placed image stays linked: editing the image updates every canvas that uses
it, and canvas exports render its current edits. Exports must be outside the
library. `command` takes one command object or an array; a canvas array applies
as one revision.

## Editor commands

These edit one asset chosen with `--asset <uuid>`. Canvas layer IDs are local to
the canvas.

```sh
spectrum images --asset <image> inspect
spectrum images --asset <image> edit --exposure -0.35 --highlights -28
spectrum images --asset <image> crop --x 0.05 --y 0.05 --width 0.9 --height 0.85
spectrum images --asset <image> curve master --points '0,0;0.4,0.55;1,1'
spectrum images --asset <image> hsl blue --saturation -20
spectrum images --asset <image> grade shadows --hue 210 --saturation 15
spectrum images --asset <image> rotate | flip --vertical | reset
spectrum images --asset <image> undo | redo | history | history-jump <revision>
spectrum canvas --asset <canvas> inspect
spectrum canvas --asset <canvas> add-text "Hello" --x 100 --y 100
spectrum canvas --asset <canvas> selection magic-wand 40 30 --tolerance 24
spectrum canvas --asset <canvas> run '{"command":"undo"}'
spectrum canvas --asset <canvas> history
```

Image commands: inspect, edit, crop, hsl, curve, grade, spot, rotate, flip,
reset, undo, redo, history, history-jump, and run. Canvas commands cover text,
images, shapes, paths, painting and Clone Stamp, selections, masks, clipping,
blend modes, layer styles and gradients, transforms, alignment, guides,
typography and fonts (`font-list --system` finds installed fonts to embed with
`font-import`), `sample` (the color at a pixel), layer copy and paste, history,
and raw `run`. `spectrum images schema` and `spectrum canvas schema` describe
each command protocol with examples.

## Benchmarks

```sh
spectrum images benchmark --strict
spectrum canvas benchmark --strict
```

Both accept `--profile hosted-ci` for shared-runner budgets. Reports separate
targets from regression budgets; `--strict` exits nonzero when a budget is missed.
