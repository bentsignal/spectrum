#!/usr/bin/env bash
set -euo pipefail
repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"
cargo build --release --locked -p spectrum-demo

case "$(uname -s)" in
  Darwin)
    bundle="$repo_root/target/dist/Spectrum.app"
    if [[ -e "$bundle" || -L "$bundle" ]]; then
      [[ -d "$bundle" && ! -L "$bundle" && "$(realpath "$bundle")" == "$bundle" ]] || exit 1
      rm -rf -- "$bundle"
    fi
    mkdir -p "$bundle/Contents/MacOS" "$bundle/Contents/Resources"
    install -m 0755 target/release/spectrum-demo "$bundle/Contents/MacOS/spectrum-demo"
    install -m 0644 packaging/spectrum-demo/Info.plist "$bundle/Contents/Info.plist"
    bash scripts/stamp-macos-bundle.sh "$bundle/Contents/Info.plist"
    bash scripts/package-macos-icon.sh assets/branding/Spectrum.icon "$bundle/Contents/Resources/Spectrum.icns"
    install -m 0644 LICENSE THIRD_PARTY.md "$bundle/Contents/Resources/"
    if [[ -n "${SPECTRUM_CODESIGN_IDENTITY:-}" ]]; then
      codesign --force --options runtime --timestamp --sign "$SPECTRUM_CODESIGN_IDENTITY" "$bundle"
      actual_team="$(codesign -dv --verbose=4 "$bundle" 2>&1 | sed -n 's/^TeamIdentifier=//p')"
      [[ "$actual_team" == "${SPECTRUM_APPLE_TEAM_ID:?missing signing team}" ]]
    else
      codesign --force --sign - "$bundle"
    fi
    codesign --verify --deep --strict "$bundle"
    "$bundle/Contents/MacOS/spectrum-demo" --version
    ;;
  Linux|MINGW*|MSYS*)
    suffix=""
    [[ "$(uname -s)" == Linux ]] || suffix=".exe"
    destination="$repo_root/target/dist/spectrum-preview"
    mkdir -p "$destination"
    install -m 0755 "target/release/spectrum-demo$suffix" "$destination/spectrum-demo$suffix"
    install -m 0644 LICENSE THIRD_PARTY.md "$destination/"
    "$destination/spectrum-demo$suffix" --version
    ;;
  *) echo "Unsupported packaging platform" >&2; exit 1 ;;
esac
