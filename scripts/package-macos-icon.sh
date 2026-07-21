#!/usr/bin/env bash
set -euo pipefail

if [[ "$#" -ne 2 ]]; then
  echo "usage: $0 <source.png> <destination.icns>" >&2
  exit 2
fi

source_png="$1"
destination="$2"
icon_root="$(mktemp -d "${TMPDIR:-/tmp}/spectrum-icon.XXXXXX")"
iconset="$icon_root/AppIcon.iconset"
trap 'rm -rf -- "$icon_root"' EXIT

mkdir -p "$iconset"
for size in 16 32 128 256 512; do
  retina_size=$((size * 2))
  sips -z "$size" "$size" "$source_png" \
    --out "$iconset/icon_${size}x${size}.png" >/dev/null
  sips -z "$retina_size" "$retina_size" "$source_png" \
    --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
done

mkdir -p "$(dirname -- "$destination")"
iconutil --convert icns "$iconset" --output "$destination"
