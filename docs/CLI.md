# CLI reference

The `lumen` binary is intended for humans, scripts, and autonomous agents. Its
stdout is always JSON after argument parsing. Operational failures are JSON on
stderr and return a nonzero status.

The portable project is selected globally:

```sh
lumen --catalog path/to/shoot.lumen <command>
```

If omitted, it defaults to `untitled.lumen` in the current directory. A `.lumen`
project is a transactional single file containing an independent revision tree
for every photo, project metadata, and content-addressed original photos. Legacy `.lumencatalog` files are preserved
and migrated to a sibling `.lumen` project when opened.

## Commands

| Command | Purpose |
| --- | --- |
| `init [NAME] [--force]` | Create or replace an empty catalog |
| `import <PATH>...` | Add supported photos |
| `list` | Return the full catalog and every photo |
| `get <ID>` | Return one photo and its edits |
| `edit <ID> [OPTIONS]` | Patch one or more adjustment values |
| `crop <ID> [OPTIONS]` | Set/clear crop and straighten |
| `hsl <ID> <COLOR> [OPTIONS]` | Edit one of eight HSL color ranges |
| `curve <ID> <CHANNEL> [OPTIONS]` | Set/reset master or RGB tone-curve points |
| `grade <ID> <RANGE> [OPTIONS]` | Color-grade shadows, midtones, or highlights |
| `spot <ID> [OPTIONS]` | Add or clear nondestructive repair-brush dabs |
| `pick <ID>... --state <STATE>` | Mark photos unmarked, keep, or reject |
| `batch-rename <ID> <NAME>` | Rename a chronological shoot batch |
| `history <ID>` | Return one photo's revision tree and session cursors |
| `history-back\|history-forward <ID>` | Move this session through one photo's history |
| `history-jump <ID> <REVISION>` | Move this session to any revision of one photo |
| `reset <ID>...` | Restore identity edits |
| `preset-list` | List reusable development presets |
| `preset-save <NAME> --from <ID>` | Save one photo's non-geometric development settings |
| `preset-apply <PRESET_ID> <ID>...` | Apply a preset while preserving each photo's crop/rotation |
| `preset-delete <PRESET_ID>` | Remove a named preset |
| `copy-edits --from ID --to ID...` | Apply one photo's complete look to others |
| `rotate <ID> [--counterclockwise]` | Rotate by 90 degrees |
| `flip <ID> [--horizontal\|--vertical]` | Toggle a flip transform |
| `remove <ID>...` | Remove catalog references, never source files |
| `export <ID> <PATH>` | Render an edited file |
| `export-batch <ID>... --directory <PATH>` | Export multiple photos as JPEG, PNG, TIFF, or WebP |
| `run '<JSON>'` | Execute one core command or an atomic command array |
| `agent start <ID> --mode <together\|separate>` | Start a collaboration scoped to one photo |
| `agent status` | Inspect the session supplied with `--session` |
| `benchmark [--strict] [--raw-import PATH]` | Measure imports, tone-curve command/preview latency, and 24 MP export |
| `schema` | Return ranges and raw-command examples |

Use `lumen <command> --help` for exact flags.

## Performance budgets

Run the deterministic release workload on any target machine:

```sh
cargo run --release -p lumen-photo --bin lumen -- benchmark
```

The JSON report separates aspirational targets from conservative regression
budgets. It measures p95 tone-curve command plus atomic catalog-save latency,
p95 1800×1200 preview rendering, a 12-photo 2400×1600 JPEG batch import, and a
complete 6000×4000 (24 MP) JPEG export. Pass `--raw-import` with a locally
accessible ARW to additionally measure the metadata-only RAW import path.
Pass `--strict` to return a nonzero exit code if a budget is missed. The default
`interactive` profile protects workstation feel. Linux CI uses
`--profile hosted-ci`, calibrated to the slower shared two-core runner with
headroom for host jitter, after building the release binary.

| Workload | Excellent target | Workstation budget | Hosted-CI budget | UX meaning |
| --- | ---: | ---: | ---: | --- |
| Portable-project load + tone-curve command + atomic 24 MP publication (p95) | 50 ms | 100 ms | 175 ms | Completion stays immediate without blocking live preview |
| 1800×1200 curve preview (p95) | 16.7 ms | 50 ms | 125 ms | Fluid locally; CI catches relative regressions on slower shared CPUs |
| 12-photo JPEG batch import (p95) | 50 ms | 250 ms | 500 ms | Import bookkeeping remains effectively immediate |
| 24 MP JPEG export | 2 s | 5 s | 5 s | Fast single export; bounded batch wait |

Source generation and benchmark warm-up are excluded from measured intervals.
The generated pixel pattern, dimensions, curve, quality, and sample counts are
fixed so results remain comparable between commits.

## Adjustment ranges

Exposure is in stops from `-5` to `5`. Temperature, tint, contrast, highlights,
shadows, whites, blacks, texture, clarity, dehaze, vibrance, saturation, and vignette use a
human-readable `-100` to `100` scale. Values are clamped by the shared core.
Sharpening and noise reduction use `0` to `100`; straighten uses `-45` to `45`
degrees. Crop coordinates are normalized from `0` to `1`.

Options that are omitted by `edit` remain unchanged:

```sh
lumen --catalog shoot.lumen edit 7 \
  --exposure -0.35 --highlights -28 --texture 12 --dehaze 8

lumen --catalog shoot.lumen crop 7 \
  --x 0.05 --y 0.05 --width 0.9 --height 0.85 --straighten 1.2
lumen --catalog shoot.lumen hsl 7 blue \
  --hue -8 --saturation 18 --luminance -4
lumen --catalog shoot.lumen curve 7 master \
  --points '0,0;0.25,0.2;0.75,0.82;1,1'
lumen --catalog shoot.lumen grade 7 highlights \
  --hue 42 --saturation 18 --luminance 4 --balance 10
lumen --catalog shoot.lumen spot 7 \
  --x 0.51 --y 0.24 --radius 0.018 --opacity 0.9
lumen --catalog shoot.lumen pick 7 8 9 --state keep
lumen --catalog shoot.lumen batch-rename 1 "Night walk"

lumen --catalog shoot.lumen preset-save "Soft color" --from 7
lumen --catalog shoot.lumen preset-apply 1 8 9 10
lumen --catalog shoot.lumen export-batch 7 8 9 10 \
  --directory exports --format jpeg --quality 90 --max-size 3200
```

## Raw commands

`run` is useful when an agent already speaks the serialized command protocol:

```sh
lumen --catalog shoot.lumen run \
  '{"command":"adjust","id":7,"patch":{"exposure":0.4,"vibrance":12}}'
```

The command names and payloads are serde-tagged kebab-case values defined by
`lumen_core::Command`. Run `lumen schema` for discoverable examples. Normal
task-oriented subcommands are preferred when shell quoting would be fragile.

## Automation rules

- Treat photo IDs as catalog-local unsigned integers.
- Read a photo with `get` before choosing relative edits.
- Imported originals are embedded immutably and deduplicated by content hash.
- Export to a new path. Lumen rejects unsupported output extensions.
- The CLI commits successful mutations to the affected photo tracks before returning success.
- Editing one photo never moves another photo's revision cursor; multi-photo commands publish their linked revisions atomically.
