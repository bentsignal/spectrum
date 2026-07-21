#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
lock_file="$repo_root/packaging/prism/macos/ghostty-proof.lock"
harness_source="$repo_root/apps/prism/native/ghostty-proof"
storage_root="$repo_root/target/ghostty-proof"
downloads_dir="$storage_root/downloads"
sources_dir="$storage_root/sources"
toolchains_dir="$storage_root/toolchains"
install_root="$storage_root/install"
scratch_root="$storage_root/build"
zig_global_cache="$storage_root/zig-global-cache"
dist_root="$storage_root/dist"

die() {
  echo "error: $*" >&2
  exit 1
}

require_command() {
  command -v "$1" >/dev/null 2>&1 || die "required command not found: $1"
}

lock_value() {
  local key="$1"
  local value
  value="$(awk -F= -v key="$key" '$1 == key { sub(/^[^=]*=/, ""); print; found = 1; exit } END { if (!found) exit 1 }' "$lock_file")" \
    || die "missing lock value: $key"
  [[ -n "$value" ]] || die "empty lock value: $key"
  printf '%s\n' "$value"
}

sha256_file() {
  shasum -a 256 "$1" | awk '{print $1}'
}

verify_file() {
  local path="$1"
  local expected="$2"
  local actual
  actual="$(sha256_file "$path")"
  [[ "$actual" == "$expected" ]] \
    || die "checksum mismatch for $path (expected $expected, got $actual)"
}

download_verified() {
  local url="$1"
  local expected="$2"
  local destination="$3"
  if [[ -f "$destination" ]]; then
    verify_file "$destination" "$expected"
    return
  fi

  local temporary="$destination.part.$$"
  curl --fail --location --proto '=https' --tlsv1.2 --output "$temporary" "$url"
  verify_file "$temporary" "$expected"
  mv -- "$temporary" "$destination"
}

safe_remove_tree() {
  local target="$1"
  case "$target" in
    "$storage_root"/*) rm -rf -- "$target" ;;
    *) die "refusing to remove path outside proof storage: $target" ;;
  esac
}

extract_once() {
  local archive="$1"
  local expected_checksum="$2"
  local top_level="$3"
  local destination="$4"
  local compression="$5"
  local marker="$destination/.spectrum-source-sha256"

  if [[ -d "$destination" ]]; then
    [[ -f "$marker" ]] || die "unverified source directory already exists: $destination"
    [[ "$(sed -n '1p' "$marker")" == "$expected_checksum" ]] \
      || die "source marker mismatch in $destination; remove that exact directory and retry"
    return
  fi

  local staging
  staging="$(mktemp -d "$storage_root/extract.XXXXXX")"
  if [[ "$compression" == "gz" ]]; then
    tar -xzf "$archive" -C "$staging"
  elif [[ "$compression" == "xz" ]]; then
    tar -xJf "$archive" -C "$staging"
  else
    die "unsupported archive compression: $compression"
  fi
  [[ -d "$staging/$top_level" ]] || die "archive did not contain expected root: $top_level"
  mv -- "$staging/$top_level" "$destination"
  printf '%s\n' "$expected_checksum" >"$marker"
  safe_remove_tree "$staging"
}

[[ -f "$lock_file" ]] || die "lock file not found: $lock_file"
[[ -d "$harness_source" ]] || die "proof harness not found: $harness_source"
[[ "$(uname -s)" == "Darwin" ]] || die "GhosttyKit proof builds require macOS"

require_command awk
require_command codesign
require_command curl
require_command ditto
require_command mktemp
require_command plutil
require_command shasum
require_command sw_vers
require_command tar
require_command xcode-select
require_command xcodebuild
require_command xcrun

xcode_developer_dir="$(xcode-select --print-path)"
case "$xcode_developer_dir" in
  *.app/Contents/Developer) ;;
  *) die "full Xcode must be active; run xcode-select with Xcode.app, not CommandLineTools" ;;
esac
xcodebuild -version >/dev/null
xcrun --sdk macosx --show-sdk-path >/dev/null \
  || die "the active Xcode is missing the macOS SDK"
xcrun --sdk iphoneos --show-sdk-path >/dev/null \
  || die "the active Xcode is missing the iOS SDK required for GhosttyKit.xcframework"
xcrun --find metal >/dev/null 2>&1 \
  || die "the active Xcode is missing the Metal toolchain (install it in Xcode Settings > Components)"
xcrun --find swift >/dev/null 2>&1 || die "the active Xcode is missing Swift"

macos_version="$(sw_vers -productVersion)"
macos_major="${macos_version%%.*}"
[[ "$macos_major" =~ ^[0-9]+$ ]] || die "could not parse macOS version: $macos_version"
(( macos_major >= 13 )) \
  || die "Ghostty 1.3.1 requires macOS 13.0 or newer (running $macos_version)"

if ! command -v msgfmt >/dev/null 2>&1; then
  if command -v brew >/dev/null 2>&1; then
    gettext_prefix="$(brew --prefix gettext 2>/dev/null || true)"
    if [[ -n "$gettext_prefix" && -x "$gettext_prefix/bin/msgfmt" ]]; then
      PATH="$gettext_prefix/bin:$PATH"
      export PATH
    fi
  fi
fi
require_command msgfmt

lock_format="$(lock_value LOCK_FORMAT)"
ghostty_version="$(lock_value GHOSTTY_VERSION)"
ghostty_tag="$(lock_value GHOSTTY_TAG)"
ghostty_revision="$(lock_value GHOSTTY_REVISION)"
ghostty_url="$(lock_value GHOSTTY_SOURCE_URL)"
ghostty_sha="$(lock_value GHOSTTY_SOURCE_SHA256)"
zig_version="$(lock_value ZIG_VERSION)"
minimum_macos="$(lock_value MINIMUM_MACOS_VERSION)"
xcframework_relative="$(lock_value GHOSTTY_XCFRAMEWORK_PATH)"
resources_relative="$(lock_value GHOSTTY_RESOURCES_PATH)"
resource_sentinel="$(lock_value GHOSTTY_RESOURCE_SENTINEL)"

[[ "$lock_format" == "1" ]] || die "unsupported lock format: $lock_format"
[[ "$ghostty_tag" == "v$ghostty_version" ]] || die "Ghostty tag/version mismatch in lock file"
[[ "$ghostty_revision" =~ ^[0-9a-f]{40}$ ]] || die "invalid Ghostty revision in lock file"
[[ "$zig_version" == "0.15.2" ]] || die "this proof expects upstream's exact Zig 0.15.2 toolchain"
[[ "$minimum_macos" == "13.0" ]] || die "this proof expects Ghostty's macOS 13.0 deployment target"
for relative_path in "$xcframework_relative" "$resources_relative" "$resource_sentinel"; do
  case "$relative_path" in
    /*|../*|*/../*|*/..) die "unsafe relative path in lock file: $relative_path" ;;
  esac
done

machine_arch="$(uname -m)"
case "$machine_arch" in
  arm64)
    zig_arch="aarch64"
    swift_arch="arm64"
    zig_url="$(lock_value ZIG_ARM64_URL)"
    zig_sha="$(lock_value ZIG_ARM64_SHA256)"
    ;;
  x86_64)
    zig_arch="x86_64"
    swift_arch="x86_64"
    zig_url="$(lock_value ZIG_X86_64_URL)"
    zig_sha="$(lock_value ZIG_X86_64_SHA256)"
    ;;
  *) die "unsupported macOS architecture: $machine_arch" ;;
esac

mkdir -p "$downloads_dir" "$sources_dir" "$toolchains_dir" \
  "$install_root" "$scratch_root" "$zig_global_cache" "$dist_root"

ghostty_archive="$downloads_dir/ghostty-$ghostty_version.tar.gz"
zig_archive="$downloads_dir/zig-$zig_arch-macos-$zig_version.tar.xz"
download_verified "$ghostty_url" "$ghostty_sha" "$ghostty_archive"
download_verified "$zig_url" "$zig_sha" "$zig_archive"

ghostty_source="$sources_dir/ghostty-$ghostty_version"
zig_source="$toolchains_dir/zig-$zig_arch-macos-$zig_version"
extract_once "$ghostty_archive" "$ghostty_sha" "ghostty-$ghostty_version" \
  "$ghostty_source" gz
extract_once "$zig_archive" "$zig_sha" "zig-$zig_arch-macos-$zig_version" \
  "$zig_source" xz

zig_binary="$zig_source/zig"
[[ -x "$zig_binary" ]] || die "verified Zig archive did not provide: $zig_binary"
actual_zig_version="$($zig_binary version)"
[[ "$actual_zig_version" == "$zig_version" ]] \
  || die "wrong Zig version after extraction (expected $zig_version, got $actual_zig_version)"

ghostty_prefix="$install_root/ghostty-$ghostty_version"
mkdir -p "$ghostty_prefix"
(
  cd "$ghostty_source"
  "$zig_binary" build -j2 \
    --prefix "$ghostty_prefix" \
    --cache-dir "$scratch_root/ghostty-zig-cache" \
    --global-cache-dir "$zig_global_cache" \
    -Doptimize=ReleaseFast \
    -Demit-xcframework=true \
    -Demit-macos-app=false
)

xcframework="$ghostty_source/$xcframework_relative"
resources="$ghostty_prefix/$resources_relative"
[[ -d "$xcframework" ]] || die "Ghostty build did not produce expected XCFramework: $xcframework"
[[ -f "$xcframework/Info.plist" ]] || die "Ghostty XCFramework has no Info.plist: $xcframework"
[[ -f "$resources/$resource_sentinel" ]] \
  || die "Ghostty resources are incomplete; missing sentinel: $resources/$resource_sentinel"
ghostty_header="$(find "$xcframework" -type f -name ghostty.h -print -quit)"
[[ -n "$ghostty_header" ]] || die "Ghostty XCFramework contains no ghostty.h"

harness_stage="$(mktemp -d "$storage_root/harness.XXXXXX")"
mkdir -p "$harness_stage/Artifacts"
cp -- "$harness_source/Package.swift" "$harness_stage/Package.swift"
cp -R -- "$harness_source/Sources" "$harness_stage/Sources"
ln -s -- "$xcframework" "$harness_stage/Artifacts/GhosttyKit.xcframework"

harness_scratch="$scratch_root/harness-$machine_arch"
xcrun swift build \
  --package-path "$harness_stage" \
  --scratch-path "$harness_scratch" \
  --configuration release \
  --arch "$swift_arch"
harness_bin_dir="$(xcrun swift build \
  --package-path "$harness_stage" \
  --scratch-path "$harness_scratch" \
  --configuration release \
  --arch "$swift_arch" \
  --show-bin-path)"
harness_executable="$harness_bin_dir/PrismGhosttyProof"
[[ -x "$harness_executable" ]] \
  || die "Swift build did not produce expected harness: $harness_executable"

bundle="$dist_root/PrismGhosttyProof.app"
bundle_staging="$(mktemp -d "$storage_root/bundle.XXXXXX")/PrismGhosttyProof.app"
mkdir -p "$bundle_staging/Contents/MacOS" "$bundle_staging/Contents/Resources"
install -m 0755 "$harness_executable" "$bundle_staging/Contents/MacOS/PrismGhosttyProof"
install -m 0644 "$harness_source/Info.plist" "$bundle_staging/Contents/Info.plist"
ditto "$resources" "$bundle_staging/Contents/Resources/ghostty"
install -m 0644 "$ghostty_source/LICENSE" \
  "$bundle_staging/Contents/Resources/GHOSTTY-LICENSE"
codesign --force --sign - "$bundle_staging"

if [[ -e "$bundle" ]]; then
  safe_remove_tree "$bundle"
fi
mv -- "$bundle_staging" "$bundle"
safe_remove_tree "$(dirname -- "$bundle_staging")"
safe_remove_tree "$harness_stage"

echo "Created source-only Ghostty proof harness:"
echo "  $bundle"
echo "Ghostty: $ghostty_tag ($ghostty_revision)"
echo "Zig: $zig_version ($machine_arch host)"
echo "Run manually when ready: open '$bundle'"
