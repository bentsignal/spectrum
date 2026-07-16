# CLI reference

The `lumen` binary is intended for humans, scripts, and autonomous agents. Its
stdout is always JSON after argument parsing. Operational failures are JSON on
stderr and return a nonzero status.

The catalog is selected globally:

```sh
lumen --catalog path/to/shoot.lumencatalog <command>
```

If omitted, it defaults to `lumen.lumencatalog` in the current directory.

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
| `history <ID>` | Return persistent history and its cursor |
| `history-back\|history-forward <ID>` | Navigate one persistent edit |
| `history-jump <ID> <INDEX>` | Restore a particular history snapshot |
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
| `run '<JSON>'` | Deserialize and execute a core command directly |
| `benchmark [--strict]` | Measure tone-curve command/preview latency and 24 MP JPEG export |
| `schema` | Return ranges and raw-command examples |

Use `lumen <command> --help` for exact flags.

## Performance budgets

Run the deterministic release workload on any target machine:

```sh
cargo run --release --bin lumen -- benchmark
```

The JSON report separates aspirational targets from conservative regression
budgets. It measures p95 tone-curve command plus atomic catalog-save latency,
p95 1800×1200 preview rendering, and a complete 6000×4000 (24 MP) JPEG export.
Pass `--strict` to return a nonzero exit code if a budget is missed. The default
`interactive` profile protects workstation feel. Linux CI uses
`--profile hosted-ci`, calibrated to the slower shared two-core runner with
headroom for host jitter, after building the release binary.

| Workload | Excellent target | Workstation budget | Hosted-CI budget | UX meaning |
| --- | ---: | ---: | ---: | --- |
| Tone-curve catalog load + command + save (p95) | 8 ms | 20 ms | 20 ms | Catalog work stays below a frame |
| 1800×1200 curve preview (p95) | 16.7 ms | 50 ms | 125 ms | Fluid locally; CI catches relative regressions on slower shared CPUs |
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
lumen --catalog shoot.lumencatalog edit 7 \
  --exposure -0.35 --highlights -28 --texture 12 --dehaze 8

lumen --catalog shoot.lumencatalog crop 7 \
  --x 0.05 --y 0.05 --width 0.9 --height 0.85 --straighten 1.2
lumen --catalog shoot.lumencatalog hsl 7 blue \
  --hue -8 --saturation 18 --luminance -4
lumen --catalog shoot.lumencatalog curve 7 master \
  --points '0,0;0.25,0.2;0.75,0.82;1,1'

lumen --catalog shoot.lumencatalog preset-save "Soft color" --from 7
lumen --catalog shoot.lumencatalog preset-apply 1 8 9 10
lumen --catalog shoot.lumencatalog export-batch 7 8 9 10 \
  --directory exports --format jpeg --quality 90 --max-size 3200
```

## Raw commands

`run` is useful when an agent already speaks the serialized command protocol:

```sh
lumen --catalog shoot.lumencatalog run \
  '{"command":"adjust","id":7,"patch":{"exposure":0.4,"vibrance":12}}'
```

The command names and payloads are serde-tagged kebab-case values defined by
`lumen_core::Command`. Run `lumen schema` for discoverable examples. Normal
task-oriented subcommands are preferred when shell quoting would be fragile.

## Automation rules

- Treat photo IDs as catalog-local unsigned integers.
- Read a photo with `get` before choosing relative edits.
- Keep the catalog under source control only if its absolute source paths are
  meaningful to collaborators.
- Export to a new path. Lumen rejects unsupported output extensions.
- The CLI persists successful mutation commands before returning success.
