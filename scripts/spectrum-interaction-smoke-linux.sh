#!/usr/bin/env bash
# Exercise Spectrum's real Linux GUI and summarize its frame trace.
set -euo pipefail

if [[ $# -lt 1 || $# -gt 2 ]]; then
  echo "usage: $0 <catalog.lumen> [trace.csv]" >&2
  exit 2
fi
for command in Xvfb xdotool python3; do
  command -v "$command" >/dev/null || { echo "missing $command" >&2; exit 2; }
done

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
catalog="$(realpath "$1")"
[[ -f "$catalog" ]] || { echo "catalog not found: $catalog" >&2; exit 2; }
trace="$(realpath -m "${2:-$repo_root/target/tmp/spectrum-interaction-smoke.csv}")"
mkdir -p "$(dirname "$trace")"
[[ -x "$repo_root/target/release/spectrum-gui" ]] || {
  echo "build target/release/spectrum-gui first" >&2
  exit 2
}

run_dir="$(mktemp -d "$repo_root/target/tmp/spectrum-smoke.XXXXXX")"
export DISPLAY=:275
unset WAYLAND_DISPLAY
export XDG_SESSION_TYPE=x11 GDK_BACKEND=x11
export XDG_CONFIG_HOME="$run_dir/config" XDG_DATA_HOME="$run_dir/data"
app_pid=
Xvfb "$DISPLAY" -screen 0 1600x1000x24 -nolisten tcp >"$run_dir/xvfb.log" 2>&1 &
xvfb_pid=$!
cleanup() {
  if [[ -n "$app_pid" ]]; then kill "$app_pid" 2>/dev/null || true; wait "$app_pid" 2>/dev/null || true; fi
  kill "$xvfb_pid" 2>/dev/null || true
  wait "$xvfb_pid" 2>/dev/null || true
}
trap cleanup EXIT
sleep 1
SPECTRUM_PERF_LOG="$trace" "$repo_root/target/release/spectrum-gui" "$catalog" >"$run_dir/app.log" 2>&1 &
app_pid=$!

ready=false
for _ in $(seq 1 40); do
  if xdotool search --onlyvisible --name Spectrum >/dev/null 2>&1; then ready=true; break; fi
  sleep 0.25
done
if [[ "$ready" != true ]]; then
  cat "$run_dir/app.log" >&2
  echo "Spectrum window did not open; logs: $run_dir" >&2
  exit 1
fi

# Coordinates assume Spectrum's 1500x940 centered window on this 1600x1000 display.
xdotool mousemove 210 260 click 1
sleep 2
xdotool mousemove 180 600 click --repeat 12 --delay 75 5
xdotool mousemove 180 600 click --repeat 12 --delay 75 4
xdotool mousemove 230 58 click 1
sleep 1
for i in $(seq 1 120); do
  xdotool mousemove $((400 + i % 2)) 400
  sleep 0.04
done
sleep 1
kill "$app_pid" 2>/dev/null || true
wait "$app_pid" 2>/dev/null || true
app_pid=

python3 - "$trace" <<'PY'
import csv
import statistics
import sys

path = sys.argv[1]
with open(path, newline="") as stream:
    rows = list(csv.DictReader(stream))
if len(rows) < 100 or not any(r["workspace"] == "canvas" for r in rows):
    raise SystemExit(f"incomplete interaction trace: {len(rows)} rows ({path})")
print(f"trace: {path} ({len(rows)} frames)")
for label, subset in (
    ("all", rows),
    ("photo scroll", [r for r in rows if r["workspace"] == "photo" and r["input"] == "scroll"]),
    ("canvas", [r for r in rows if r["workspace"] == "canvas"]),
):
    if subset:
        ui = statistics.median(float(r["ui_ms"]) for r in subset)
        inactive = statistics.median(float(r["inactive_ms"]) for r in subset)
        print(f"{label}: {len(subset)} frames, median UI {ui:.3f} ms, inactive {inactive:.3f} ms")
PY
