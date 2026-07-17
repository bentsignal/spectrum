#!/usr/bin/env bash
set -euo pipefail

cargo build --release --locked -p lumen-photo --bins
bundle="target/dist/Lumen.app"
rm -rf "$bundle"
mkdir -p "$bundle/Contents/MacOS" "$bundle/Contents/Resources"
cp target/release/lumen-gui "$bundle/Contents/MacOS/lumen-gui"
cp packaging/macos/Info.plist "$bundle/Contents/Info.plist"
cp THIRD_PARTY.md "$bundle/Contents/Resources/THIRD_PARTY.md"
cp target/release/lumen "target/dist/lumen-macos"
codesign --force --deep --sign - "$bundle"
echo "Created $bundle and target/dist/lumen-macos"
