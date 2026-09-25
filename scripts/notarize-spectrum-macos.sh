#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd -P -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
bundle="$repo_root/target/dist/Spectrum.app"
archive="$repo_root/target/dist/Spectrum-macos-notarized.zip"
apple_id="${SPECTRUM_APPLE_ID:?set SPECTRUM_APPLE_ID}"
team_id="${SPECTRUM_APPLE_TEAM_ID:?set SPECTRUM_APPLE_TEAM_ID}"
app_password="${SPECTRUM_APPLE_APP_PASSWORD:?set SPECTRUM_APPLE_APP_PASSWORD}"
[[ -d "$bundle" && ! -L "$bundle" ]] || { echo "missing Spectrum.app" >&2; exit 1; }
[[ ! -e "$archive" && ! -L "$archive" ]] || rm -f -- "$archive"

submission="$(mktemp -d "${RUNNER_TEMP:-${TMPDIR:-/tmp}}/spectrum-notary.XXXXXX")"
trap 'rm -rf -- "$submission"' EXIT
ditto -c -k --keepParent "$bundle" "$submission/Spectrum.zip"
xcrun notarytool submit "$submission/Spectrum.zip" \
  --apple-id "$apple_id" --team-id "$team_id" --password "$app_password" --wait
xcrun stapler staple "$bundle"
xcrun stapler validate "$bundle"
codesign --verify --deep --strict --verbose=2 "$bundle"
spctl --assess --type execute --verbose=2 "$bundle"
ditto -c -k --keepParent "$bundle" "$archive"
echo "Created notarized $archive"
