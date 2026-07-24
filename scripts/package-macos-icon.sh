#!/usr/bin/env bash
set -euo pipefail

if [[ "$#" -ne 2 ]]; then
  echo "usage: $0 <source.icon> <destination.icns>" >&2
  exit 2
fi

source_icon="$1"
destination="$2"
icon_root="$(mktemp -d "${TMPDIR:-/tmp}/spectrum-icon.XXXXXX")"
trap 'rm -rf -- "$icon_root"' EXIT

[[ -d "$source_icon" && "$source_icon" == *.icon ]] || {
  echo "native macOS icon source must be an Icon Composer .icon package: $source_icon" >&2
  exit 1
}

icon_name="$(basename -- "$source_icon" .icon)"
destination_name="$(basename -- "$destination" .icns)"
[[ "$icon_name" == "$destination_name" ]] || {
  echo "icon source and destination names must match: $icon_name != $destination_name" >&2
  exit 1
}

compiled="$icon_root/compiled"
partial_plist="$icon_root/partial.plist"
mkdir -p "$compiled"
xcrun actool \
  --compile "$compiled" \
  --platform macosx \
  --minimum-deployment-target 11.0 \
  --target-device mac \
  --app-icon "$icon_name" \
  --standalone-icon-behavior all \
  --output-partial-info-plist "$partial_plist" \
  --warnings \
  --errors \
  --notices \
  "$source_icon" >/dev/null

[[ -f "$compiled/$icon_name.icns" && -f "$compiled/Assets.car" ]] || {
  echo "actool did not produce both legacy and native icon resources" >&2
  exit 1
}
[[ "$(plutil -extract CFBundleIconFile raw -o - "$partial_plist")" == "$icon_name" ]]
[[ "$(plutil -extract CFBundleIconName raw -o - "$partial_plist")" == "$icon_name" ]]

mkdir -p "$(dirname -- "$destination")"
install -m 0644 "$compiled/$icon_name.icns" "$destination"
install -m 0644 "$compiled/Assets.car" "$(dirname -- "$destination")/Assets.car"
