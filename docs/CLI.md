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
| `reset <ID>...` | Restore identity edits |
| `copy-edits --from ID --to ID...` | Apply one photo's complete look to others |
| `rotate <ID> [--counterclockwise]` | Rotate by 90 degrees |
| `flip <ID> [--horizontal\|--vertical]` | Toggle a flip transform |
| `remove <ID>...` | Remove catalog references, never source files |
| `export <ID> <PATH>` | Render an edited file |
| `run '<JSON>'` | Deserialize and execute a core command directly |
| `schema` | Return ranges and raw-command examples |

Use `lumen <command> --help` for exact flags.

## Adjustment ranges

Exposure is in stops from `-5` to `5`. Temperature, tint, contrast, highlights,
shadows, whites, blacks, clarity, vibrance, saturation, and vignette use a
human-readable `-100` to `100` scale. Values are clamped by the shared core.

Options that are omitted by `edit` remain unchanged:

```sh
lumen --catalog shoot.lumencatalog edit 7 \
  --exposure -0.35 --highlights -28 --shadows 12 --vignette -8
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

