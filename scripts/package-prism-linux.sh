#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

cargo build --release --locked -p prism --bins

destination="$repo_root/target/dist/prism-linux"
rm -rf -- "$destination"
mkdir -p "$destination"
install -m 0755 "$repo_root/target/release/prism" "$destination/prism"
install -m 0755 "$repo_root/target/release/prism-gui" "$destination/prism-gui"
install -m 0644 "$repo_root/LICENSE" "$destination/LICENSE"
install -m 0644 "$repo_root/THIRD_PARTY.md" "$destination/THIRD_PARTY.md"
install -m 0644 "$repo_root/assets/branding/prism-app-icon.png" \
  "$destination/com.bentsignal.Prism.png"
install -m 0644 "$repo_root/packaging/prism/linux/com.bentsignal.Prism.desktop" \
  "$destination/com.bentsignal.Prism.desktop"

echo "Created $destination"
