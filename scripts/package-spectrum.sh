#!/usr/bin/env bash
set -euo pipefail
repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"
# macOS and Linux packages carry FFmpeg inside the binary; Windows carries
# the LGPL build named by FFMPEG_DIR beside it.
features=()
case "$(uname -s)" in
  Darwin|Linux) features=(--features spectrum-desktop/static-ffmpeg,spectrum/static-ffmpeg) ;;
esac
cargo build --release --locked -p spectrum-desktop -p spectrum "${features[@]}"

case "$(uname -s)" in
  Darwin)
    bundle="$repo_root/target/dist/Spectrum.app"
    if [[ -e "$bundle" || -L "$bundle" ]]; then
      [[ -d "$bundle" && ! -L "$bundle" && "$(realpath "$bundle")" == "$bundle" ]] || exit 1
      rm -rf -- "$bundle"
    fi
    mkdir -p "$bundle/Contents/MacOS" "$bundle/Contents/Resources"
    # The app is also its command line when started as `spectrum`, so the
    # command agents and terminals use is a link to it.
    install -m 0755 target/release/spectrum-desktop "$bundle/Contents/MacOS/spectrum-desktop"
    ln -s spectrum-desktop "$bundle/Contents/MacOS/spectrum"
    install -m 0644 packaging/macos/Info.plist "$bundle/Contents/Info.plist"
    bash scripts/stamp-macos-bundle.sh "$bundle/Contents/Info.plist"
    bash scripts/package-macos-icon.sh assets/branding/Spectrum.icon "$bundle/Contents/Resources/Spectrum.icns"
    install -m 0644 LICENSE THIRD_PARTY.md packaging/licenses/UBUNTU-FONT-LICENCE-1.0.txt "$bundle/Contents/Resources/"
    if [[ -n "${SPECTRUM_CODESIGN_IDENTITY:-}" ]]; then
      codesign --force --options runtime --timestamp --sign "$SPECTRUM_CODESIGN_IDENTITY" "$bundle"
      actual_team="$(codesign -dv --verbose=4 "$bundle" 2>&1 | sed -n 's/^TeamIdentifier=//p')"
      [[ "$actual_team" == "${SPECTRUM_APPLE_TEAM_ID:?missing signing team}" ]]
    else
      codesign --force --sign - "$bundle"
    fi
    codesign --verify --deep --strict "$bundle"
    "$bundle/Contents/MacOS/spectrum-desktop" --version
    "$bundle/Contents/MacOS/spectrum" --version
    ;;
  Linux|MINGW*|MSYS*)
    suffix=""
    [[ "$(uname -s)" == Linux ]] || suffix=".exe"
    destination="$repo_root/target/dist/spectrum"
    mkdir -p "$destination"
    install -m 0755 "target/release/spectrum-desktop$suffix" "$destination/spectrum-desktop$suffix"
    if [[ -z "$suffix" ]]; then
      ln -sf spectrum-desktop "$destination/spectrum"
    else
      # Windows links need extra rights, so it keeps its own command line.
      install -m 0755 "target/release/spectrum$suffix" "$destination/spectrum$suffix"
    fi
    install -m 0644 LICENSE THIRD_PARTY.md packaging/licenses/UBUNTU-FONT-LICENCE-1.0.txt "$destination/"
    if [[ -n "$suffix" ]]; then
      install -m 0644 "${FFMPEG_DIR:?FFMPEG_DIR names the FFmpeg build Windows packages carry}"/bin/*.dll "$destination/"
      install -m 0644 "$FFMPEG_DIR/LICENSE.txt" "$destination/FFMPEG-LICENSE.txt"
    fi
    "$destination/spectrum-desktop$suffix" --version
    "$destination/spectrum$suffix" --version
    ;;
  *) echo "Unsupported packaging platform" >&2; exit 1 ;;
esac
