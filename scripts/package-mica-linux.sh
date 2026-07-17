#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

cargo build --release --locked -p mica --bins

destination="$repo_root/target/dist/mica-linux"
rm -rf -- "$destination"
mkdir -p "$destination"
install -m 0755 "$repo_root/target/release/mica" "$destination/mica"
install -m 0755 "$repo_root/target/release/mica-gui" "$destination/mica-gui"
install -m 0644 "$repo_root/LICENSE" "$destination/LICENSE"
install -m 0644 "$repo_root/THIRD_PARTY.md" "$destination/THIRD_PARTY.md"
install -m 0644 "$repo_root/packaging/mica/linux/com.bentsignal.Mica.desktop" \
  "$destination/com.bentsignal.Mica.desktop"

echo "Created $destination"
