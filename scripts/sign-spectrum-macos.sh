#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd -P -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
bundle="$repo_root/target/dist/Spectrum.app"
identity="${SPECTRUM_CODESIGN_IDENTITY:?set SPECTRUM_CODESIGN_IDENTITY to a Developer ID Application identity}"
[[ -d "$bundle" && ! -L "$bundle" ]] || { echo "missing Spectrum.app" >&2; exit 1; }

sign_code() {
  local identifier="${2:-}"
  local args=(--force --options runtime --timestamp --sign "$identity")
  if [[ -n "$identifier" ]]; then args+=(--identifier "$identifier"); fi
  codesign "${args[@]}" "$1"
  codesign --verify --strict --verbose=2 "$1"
}

# Sign nested code before sealing the outer app bundle.
if [[ -f "$bundle/Contents/Frameworks/libSpectrumGhosttyBridge.dylib" ]]; then
  sign_code "$bundle/Contents/Frameworks/libSpectrumGhosttyBridge.dylib" \
    com.bentsignal.spectrum.ghostty-bridge
fi
sign_code "$bundle/Contents/MacOS/spectrum" com.bentsignal.spectrum.cli
sign_code "$bundle"
sign_code "$repo_root/target/dist/spectrum-macos" com.bentsignal.spectrum.cli
codesign --verify --deep --strict --verbose=2 "$bundle"

team_id="${SPECTRUM_APPLE_TEAM_ID:-}"
if [[ -n "$team_id" ]]; then
  actual_team="$(codesign -dv --verbose=4 "$bundle" 2>&1 | sed -n 's/^TeamIdentifier=//p')"
  [[ "$actual_team" == "$team_id" ]] || {
    echo "signed app TeamIdentifier does not match SPECTRUM_APPLE_TEAM_ID" >&2
    exit 1
  }
fi
