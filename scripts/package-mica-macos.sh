#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

cargo build --release --locked -p mica --bins

bundle="$repo_root/target/dist/Mica.app"
rm -rf -- "$bundle"
mkdir -p "$bundle/Contents/MacOS" "$bundle/Contents/Resources"
install -m 0755 "$repo_root/target/release/mica-gui" "$bundle/Contents/MacOS/mica-gui"
install -m 0644 "$repo_root/packaging/mica/macos/Info.plist" "$bundle/Contents/Info.plist"
install -m 0644 "$repo_root/LICENSE" "$bundle/Contents/Resources/LICENSE"
install -m 0644 "$repo_root/THIRD_PARTY.md" "$bundle/Contents/Resources/THIRD_PARTY.md"

cli="$repo_root/target/dist/mica-macos"
install -m 0755 "$repo_root/target/release/mica" "$cli"

codesign --force --deep --sign - "$bundle"
echo "Created $bundle and $cli"
