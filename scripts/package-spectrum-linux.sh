#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

cargo build --release --locked -p spectrum --bins
destination="$repo_root/target/dist/spectrum-linux"
if [[ -e "$destination" || -L "$destination" ]]; then
  [[ -d "$destination" && ! -L "$destination" \
    && "$(realpath "$destination")" == "$destination" ]] || {
    echo "refusing to replace unsafe package path: $destination" >&2
    exit 1
  }
  rm -rf -- "$destination"
fi
mkdir -p "$destination"
install -m 0755 "$repo_root/target/release/spectrum-gui" "$destination/spectrum-gui"
install -m 0755 "$repo_root/target/release/spectrum" "$destination/spectrum"
install -m 0644 "$repo_root/LICENSE" "$destination/LICENSE"
install -m 0644 "$repo_root/THIRD_PARTY.md" "$destination/THIRD_PARTY.md"
install -m 0644 "$repo_root/packaging/prism/licenses/UBUNTU-FONT-LICENCE-1.0.txt" \
  "$destination/UBUNTU-FONT-LICENCE-1.0.txt"
install -m 0644 "$repo_root/assets/branding/prism-app-icon.png" \
  "$destination/com.bentsignal.Spectrum.png"
install -m 0644 "$repo_root/packaging/spectrum/linux/com.bentsignal.Spectrum.desktop" \
  "$destination/com.bentsignal.Spectrum.desktop"
echo "Created $destination"
