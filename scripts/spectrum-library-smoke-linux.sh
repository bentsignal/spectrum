#!/usr/bin/env bash
# Real desktop + CLI integration, in an isolated library. Requires Xvfb and xdotool.
set -euo pipefail
repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"
bin_dir="${SPECTRUM_TEST_BIN_DIR:-$repo_root/target/release}"
for tool in Xvfb xdotool python3; do command -v "$tool" >/dev/null; done
mkdir -p target/tmp
review_dir="$(mktemp -d "$repo_root/target/tmp/library-smoke.XXXXXX")"
export XDG_DATA_HOME="$review_dir/data" XDG_CONFIG_HOME="$review_dir/config"
export SPECTRUM_LIBRARY="$review_dir/library"
unset WAYLAND_DISPLAY WAYLAND_SOCKET
Xvfb -displayfd 3 -screen 0 1600x1000x24 -nolisten tcp 3>"$review_dir/display" >"$review_dir/xvfb.log" 2>&1 &
xvfb_pid=$!
app_pid=
trap 'if [[ -n "$app_pid" ]]; then kill "$app_pid" 2>/dev/null || true; fi; kill "$xvfb_pid" 2>/dev/null || true' EXIT
for _ in $(seq 1 80); do [[ -s "$review_dir/display" ]] && break; sleep .1; done
export DISPLAY=":$(cat "$review_dir/display")"
python3 - "$review_dir/input.png" <<'PY'
import struct, sys, zlib
chunk=lambda k,b: struct.pack('>I',len(b))+k+b+struct.pack('>I',zlib.crc32(k+b))
raw=(b'\0'+bytes([64,64,64])*512)*512
with open(sys.argv[1],'wb') as f:
    f.write(b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR',struct.pack('>IIBBBBB',512,512,8,2,0,0,0))+chunk(b'IDAT',zlib.compress(raw))+chunk(b'IEND',b''))
PY
cli() { "$bin_dir/spectrum" "$@"; }
field() { python3 -c 'import json,sys;x=json.load(open(sys.argv[1]));print((x[0] if isinstance(x,list) else x)[sys.argv[2]])' "$1" "$2"; }
cli images import "$review_dir/input.png" >"$review_dir/image.json"
cli canvas new Linked --width 512 --height 512 >"$review_dir/canvas.json"
image_id="$(field "$review_dir/image.json" id)"
canvas_id="$(field "$review_dir/canvas.json" id)"
cli canvas place "$canvas_id" "$image_id" >"$review_dir/place.json"
cli copy "$canvas_id" >"$review_dir/copy.json"
copy_id="$(field "$review_dir/copy.json" id)"
cli canvas export "$canvas_id" "$review_dir/before.png" >/dev/null
"$bin_dir/spectrum-gui" "$SPECTRUM_LIBRARY/$(field "$review_dir/canvas.json" document)" >"$review_dir/app.log" 2>&1 &
app_pid=$!
for _ in $(seq 1 80); do xdotool search --onlyvisible --name Spectrum >/dev/null 2>&1 && break; sleep .25; done
sleep 2
# Open the selected linked image in Photos via the new desktop control.
xdotool mousemove 510 45 click 1
sleep 2
cli images adjust "$image_id" '{"exposure":1.0}' >"$review_dir/live-image.json"
python3 - "$review_dir/live-image.json" <<'PY'
import json,sys
r=json.load(open(sys.argv[1]))
assert r['kind']=='applied' and r['committed_revision'],r
PY
xdotool mousemove 235 45 click 1
sleep 1
cli canvas place "$canvas_id" "$image_id" >"$review_dir/live-place.json"
cli canvas export "$canvas_id" "$review_dir/after.png" >/dev/null
cli canvas export "$copy_id" "$review_dir/copy.png" >/dev/null
python3 - "$review_dir" <<'PY'
from pathlib import Path
import json,sys
p=Path(sys.argv[1])
assert (p/'before.png').read_bytes()!=(p/'after.png').read_bytes()
assert (p/'before.png').read_bytes()==(p/'copy.png').read_bytes()
assert json.loads((p/'live-place.json').read_text())['layer']==2
print('PASS: desktop image navigation, live image edits, live canvas placement, independent deep copy, and current exports')
PY
printf 'Evidence: %s\n' "$review_dir"
