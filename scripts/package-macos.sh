#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

cargo build --release --locked -p lumen-photo --bins
bundle="$repo_root/target/dist/Lumen.app"
rm -rf -- "$bundle"
mkdir -p "$bundle/Contents/MacOS" "$bundle/Contents/Resources"
install -m 0755 "$repo_root/target/release/lumen-gui" "$bundle/Contents/MacOS/lumen-gui"
install -m 0755 "$repo_root/target/release/lumen" "$bundle/Contents/MacOS/lumen"
install -m 0644 "$repo_root/packaging/macos/Info.plist" "$bundle/Contents/Info.plist"
"$repo_root/scripts/package-macos-icon.sh" \
  "$repo_root/assets/branding/lumen-app-icon.png" \
  "$bundle/Contents/Resources/Lumen.icns"
install -m 0644 "$repo_root/LICENSE" "$bundle/Contents/Resources/LICENSE"
install -m 0644 "$repo_root/THIRD_PARTY.md" "$bundle/Contents/Resources/THIRD_PARTY.md"
install -m 0755 "$repo_root/target/release/lumen" "$repo_root/target/dist/lumen-macos"
codesign --force --deep --sign - "$bundle"
echo "Created $bundle and target/dist/lumen-macos"
