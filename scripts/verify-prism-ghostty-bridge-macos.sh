#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd -P -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
metadata_validator="$repo_root/scripts/validate-prism-ghostty-metadata.py"

[[ $# -ge 3 ]] || {
  echo "usage: $0 <bridge-dylib> <minimum-macos> <architecture> [forbidden-path ...]" >&2
  exit 2
}

bridge="$1"
minimum_macos="$2"
expected_arch="$3"
shift 3
forbidden_paths=("$@")

die() {
  echo "error: $*" >&2
  exit 1
}

[[ "$bridge" == /* && -f "$bridge" && ! -L "$bridge" ]] \
  || die "bridge must be an absolute, regular, non-symlink file: $bridge"
[[ "$(realpath "$bridge")" == "$bridge" ]] \
  || die "bridge path contains a symlink or non-canonical component: $bridge"
[[ "$minimum_macos" =~ ^[0-9]+\.[0-9]+$ ]] || die "invalid minimum macOS version"
[[ "$expected_arch" == "arm64" || "$expected_arch" == "x86_64" ]] \
  || die "unsupported bridge architecture: $expected_arch"
[[ -f "$metadata_validator" && ! -L "$metadata_validator" ]] \
  || die "metadata validator not found: $metadata_validator"
nm_tool="$(command -v nm)" || die "required command not found: nm"

install_name="$(otool -D "$bridge" | sed -n '2p')"
[[ "$install_name" == "@rpath/libPrismGhosttyBridge.dylib" ]] \
  || die "Ghostty bridge has an unexpected install name: ${install_name:-missing}"

bridge_dependencies="$(otool -L "$bridge" | tail -n +2)"
while IFS= read -r dependency; do
  dependency="${dependency#${dependency%%[![:space:]]*}}"
  dependency="${dependency%% *}"
  [[ -n "$dependency" ]] || continue
  case "$dependency" in
    @rpath/libPrismGhosttyBridge.dylib | \
    /System/Library/Frameworks/AppKit.framework/* | \
    /System/Library/Frameworks/Carbon.framework/* | \
    /System/Library/Frameworks/CoreFoundation.framework/* | \
    /System/Library/Frameworks/CoreGraphics.framework/* | \
    /System/Library/Frameworks/CoreText.framework/* | \
    /System/Library/Frameworks/CoreVideo.framework/* | \
    /System/Library/Frameworks/Foundation.framework/* | \
    /System/Library/Frameworks/IOSurface.framework/* | \
    /System/Library/Frameworks/Metal.framework/* | \
    /System/Library/Frameworks/QuartzCore.framework/* | \
    /usr/lib/libSystem.B.dylib | \
    /usr/lib/libc++.1.dylib | \
    /usr/lib/libobjc.A.dylib | \
    /usr/lib/libz.1.dylib | \
    /usr/lib/swift/libswift*.dylib) ;;
    *) die "Ghostty bridge has a non-allowlisted dependency: $dependency" ;;
  esac
done <<<"$bridge_dependencies"

bridge_arches="$(lipo -archs "$bridge")"
[[ " $bridge_arches " == *" $expected_arch "* ]] \
  || die "Ghostty bridge does not contain architecture $expected_arch"
otool -l "$bridge" | awk -v expected="$minimum_macos" '
  $1 == "cmd" && $2 == "LC_BUILD_VERSION" { in_build = 1; next }
  in_build && $1 == "minos" { found = ($2 == expected); exit }
  END { exit found ? 0 : 1 }
' || die "Ghostty bridge deployment target is not macOS $minimum_macos"

python3 "$metadata_validator" symbols \
  "$nm_tool" \
  "$bridge" \
  "$expected_arch" \
  _prism_ghostty_bridge_abi_version \
  _prism_ghostty_global_init \
  _prism_ghostty_runtime_create \
  _prism_ghostty_runtime_tick \
  _prism_ghostty_runtime_set_focus \
  _prism_ghostty_runtime_destroy \
  _prism_ghostty_surface_create \
  _prism_ghostty_surface_set_state \
  _prism_ghostty_surface_edit \
  _prism_ghostty_surface_request_close \
  _prism_ghostty_surface_destroy

for forbidden in "${forbidden_paths[@]}"; do
  [[ -n "$forbidden" ]] || continue
  if strings "$bridge" | grep -F -- "$forbidden" >/dev/null; then
    die "Ghostty bridge retained forbidden build path: $forbidden"
  fi
done
codesign --verify --strict "$bridge"
