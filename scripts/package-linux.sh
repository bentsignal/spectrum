#!/usr/bin/env bash
set -euo pipefail

cargo build --release --bins --locked
destination="target/dist/lumen-linux"
rm -rf "$destination"
mkdir -p "$destination"
cp target/release/lumen-gui target/release/lumen THIRD_PARTY.md "$destination/"
echo "Created $destination"
