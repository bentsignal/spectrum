#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

cargo build --release --locked -p lumen-photo --bins
destination="$repo_root/target/dist/lumen-linux"
rm -rf -- "$destination"
mkdir -p "$destination"
install -m 0755 "$repo_root/target/release/lumen-gui" "$destination/lumen-gui"
install -m 0755 "$repo_root/target/release/lumen" "$destination/lumen"
install -m 0644 "$repo_root/THIRD_PARTY.md" "$destination/THIRD_PARTY.md"
install -m 0644 "$repo_root/assets/branding/lumen-app-icon.png" \
  "$destination/com.bentsignal.Lumen.png"
install -m 0644 "$repo_root/packaging/lumen/linux/com.bentsignal.Lumen.desktop" \
  "$destination/com.bentsignal.Lumen.desktop"
echo "Created $destination"
