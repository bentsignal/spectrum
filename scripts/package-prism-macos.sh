#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

cargo build --release --locked -p prism --bins

bundle="$repo_root/target/dist/Prism.app"
rm -rf -- "$bundle"
mkdir -p "$bundle/Contents/MacOS" "$bundle/Contents/Resources"
install -m 0755 "$repo_root/target/release/prism-gui" "$bundle/Contents/MacOS/prism-gui"
install -m 0755 "$repo_root/target/release/prism" "$bundle/Contents/MacOS/prism"
install -m 0644 "$repo_root/packaging/prism/macos/Info.plist" "$bundle/Contents/Info.plist"
"$repo_root/scripts/package-macos-icon.sh" \
  "$repo_root/assets/branding/cropped-prism.png" \
  "$bundle/Contents/Resources/Prism.icns"
install -m 0644 "$repo_root/LICENSE" "$bundle/Contents/Resources/LICENSE"
install -m 0644 "$repo_root/THIRD_PARTY.md" "$bundle/Contents/Resources/THIRD_PARTY.md"

cli="$repo_root/target/dist/prism-macos"
install -m 0755 "$repo_root/target/release/prism" "$cli"

codesign --force --deep --sign - "$bundle"
echo "Created $bundle and $cli"
