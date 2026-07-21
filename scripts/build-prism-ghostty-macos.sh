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

sdk_root_supports_arm64() {
  local libsystem_tbd="$1"
  awk '
    /^targets:[[:space:]]/ { in_root_targets = 1 }
    in_root_targets && /arm64-macos/ { found = 1 }
    in_root_targets && /^install-name:/ { exit }
    END { exit found ? 0 : 1 }
  ' "$libsystem_tbd"
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
require_command ln
require_command mktemp
require_command nm
require_command plutil
require_command shasum
require_command sw_vers
require_command tar
require_command xcode-select
require_command xcodebuild
require_command xcrun

xcode_developer_dir="${DEVELOPER_DIR:-$(xcode-select --print-path)}"
case "$xcode_developer_dir" in
  *.app/Contents/Developer) ;;
  *) die "full Xcode must be active through DEVELOPER_DIR or xcode-select, not CommandLineTools" ;;
esac
xcode_version_output="$(xcodebuild -version)"
xcode_version="$(printf '%s\n' "$xcode_version_output" | awk 'NR == 1 { print $2; exit }')"
xcode_build="$(printf '%s\n' "$xcode_version_output" | awk 'NR == 2 { print $3; exit }')"
[[ -n "$xcode_version" && -n "$xcode_build" ]] \
  || die "could not identify the active Xcode version"
macos_sdk="$(xcrun --sdk macosx --show-sdk-path)" \
  || die "the active Xcode is missing the macOS SDK"
[[ -d "$macos_sdk" ]] || die "the active macOS SDK path does not exist: $macos_sdk"
xcrun --sdk iphoneos --show-sdk-path >/dev/null \
  || die "the active Xcode is missing the iOS SDK required for GhosttyKit.xcframework"
if ! xcrun --sdk macosx metal -v >/dev/null 2>&1; then
  die "the active Xcode cannot run the Metal compiler; install the component with: xcodebuild -downloadComponent MetalToolchain"
fi
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
ghostty_tag_object="$(lock_value GHOSTTY_TAG_OBJECT)"
ghostty_revision="$(lock_value GHOSTTY_REVISION)"
ghostty_url="$(lock_value GHOSTTY_SOURCE_URL)"
ghostty_sha="$(lock_value GHOSTTY_SOURCE_SHA256)"
zig_version="$(lock_value ZIG_VERSION)"
homebrew_zig_formula="$(lock_value HOMEBREW_ZIG_FORMULA)"
homebrew_zig_arm64_path="$(lock_value HOMEBREW_ZIG_ARM64_PATH)"
homebrew_llvm_libtool_path="$(lock_value HOMEBREW_LLVM_LIBTOOL_PATH)"
minimum_macos="$(lock_value MINIMUM_MACOS_VERSION)"
xcframework_relative="$(lock_value GHOSTTY_XCFRAMEWORK_PATH)"
resources_relative="$(lock_value GHOSTTY_RESOURCES_PATH)"
resource_sentinel="$(lock_value GHOSTTY_RESOURCE_SENTINEL)"

[[ "$lock_format" == "1" ]] || die "unsupported lock format: $lock_format"
[[ "$ghostty_tag" == "v$ghostty_version" ]] || die "Ghostty tag/version mismatch in lock file"
[[ "$ghostty_tag_object" =~ ^[0-9a-f]{40}$ ]] \
  || die "invalid Ghostty annotated tag object in lock file"
[[ "$ghostty_revision" =~ ^[0-9a-f]{40}$ ]] \
  || die "invalid Ghostty peeled source revision in lock file"
[[ "$ghostty_tag_object" != "$ghostty_revision" ]] \
  || die "Ghostty annotated tag object and peeled source revision must be distinct"
[[ "$zig_version" == "0.15.2" ]] || die "this proof expects upstream's exact Zig 0.15.2 toolchain"
[[ "$homebrew_zig_formula" == "zig@0.15" ]] \
  || die "this proof expects Homebrew's zig@0.15 patched toolchain formula"
[[ "$homebrew_zig_arm64_path" == "/opt/homebrew/opt/zig@0.15/bin/zig" ]] \
  || die "unexpected Homebrew Zig path in lock file: $homebrew_zig_arm64_path"
[[ "$homebrew_llvm_libtool_path" == "/opt/homebrew/opt/llvm@20/bin/llvm-libtool-darwin" ]] \
  || die "unexpected Homebrew LLVM libtool path in lock file: $homebrew_llvm_libtool_path"
[[ "$minimum_macos" == "13.0" ]] || die "this proof expects Ghostty's macOS 13.0 deployment target"
for relative_path in "$xcframework_relative" "$resources_relative" "$resource_sentinel"; do
  case "$relative_path" in
    /*|../*|*/../*|*/..) die "unsafe relative path in lock file: $relative_path" ;;
  esac
done

machine_arch="$(uname -m)"
zig_source_kind="official"
zig_binary=""
case "$machine_arch" in
  arm64)
    swift_arch="arm64"
    macos_libsystem_tbd="$macos_sdk/usr/lib/libSystem.tbd"
    [[ -f "$macos_libsystem_tbd" ]] \
      || die "the active macOS SDK has no libSystem.tbd: $macos_libsystem_tbd"
    if sdk_root_supports_arm64 "$macos_libsystem_tbd"; then
      zig_arch="aarch64"
      zig_url="$(lock_value ZIG_ARM64_URL)"
      zig_sha="$(lock_value ZIG_ARM64_SHA256)"
    else
      # Official Zig 0.15.2 cannot parse the arm64e-only root TBD labels in
      # current SDKs. Ghostty #11991 recommends Homebrew's patched 0.15.2
      # bottle. Do not fall back to the official x86_64 binary under Rosetta;
      # that workaround fails while building libc++ on newer SDKs.
      zig_arch="aarch64"
      zig_source_kind="homebrew"
      zig_binary="$homebrew_zig_arm64_path"
      homebrew_zig_install="brew install --force-bottle $homebrew_zig_formula"
      command -v brew >/dev/null 2>&1 \
        || die "the active SDK requires Homebrew's patched Zig $zig_version; install it with: $homebrew_zig_install"
      command -v realpath >/dev/null 2>&1 \
        || die "realpath is required to validate the Homebrew Zig bottle"
      actual_homebrew_prefix="$(brew --prefix "$homebrew_zig_formula" 2>/dev/null || true)"
      expected_homebrew_prefix="${homebrew_zig_arm64_path%/bin/zig}"
      [[ "$actual_homebrew_prefix" == "$expected_homebrew_prefix" ]] \
        || die "the active SDK requires $homebrew_zig_formula at $expected_homebrew_prefix; install it with: $homebrew_zig_install"
      [[ -x "$zig_binary" ]] \
        || die "the active SDK requires Homebrew's patched Zig $zig_version; install it with: $homebrew_zig_install"
      resolved_homebrew_prefix="$(realpath "$actual_homebrew_prefix")"
      resolved_zig_binary="$(realpath "$zig_binary")"
      expected_resolved_zig_binary="$(realpath "$resolved_homebrew_prefix/bin/zig")"
      [[ "$resolved_zig_binary" == "$expected_resolved_zig_binary" ]] \
        || die "Homebrew Zig resolves outside the installed $homebrew_zig_formula keg; reinstall it with: $homebrew_zig_install"
      homebrew_receipt="$resolved_homebrew_prefix/INSTALL_RECEIPT.json"
      [[ -f "$homebrew_receipt" ]] \
        || die "Homebrew Zig has no installation receipt; reinstall the bottle with: $homebrew_zig_install"
      poured_from_bottle="$(plutil -extract poured_from_bottle raw -o - "$homebrew_receipt" 2>/dev/null || true)"
      [[ "$poured_from_bottle" == "true" ]] \
        || die "Homebrew Zig was not poured from a bottle; reinstall it with: $homebrew_zig_install"
      actual_zig_version="$("$zig_binary" version)"
      [[ "$actual_zig_version" == "$zig_version" ]] \
        || die "wrong Homebrew Zig version at $zig_binary (expected $zig_version, got $actual_zig_version); install the exact bottle with: $homebrew_zig_install"
      [[ -x "$homebrew_llvm_libtool_path" ]] \
        || die "Homebrew Zig's LLVM archive tool is missing: $homebrew_llvm_libtool_path"
    fi
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
download_verified "$ghostty_url" "$ghostty_sha" "$ghostty_archive"

ghostty_source="$sources_dir/ghostty-$ghostty_version"
extract_once "$ghostty_archive" "$ghostty_sha" "ghostty-$ghostty_version" \
  "$ghostty_source" gz
if [[ "$zig_source_kind" == "official" ]]; then
  zig_archive="$downloads_dir/zig-$zig_arch-macos-$zig_version.tar.xz"
  download_verified "$zig_url" "$zig_sha" "$zig_archive"
  zig_source="$toolchains_dir/zig-$zig_arch-macos-$zig_version"
  extract_once "$zig_archive" "$zig_sha" "zig-$zig_arch-macos-$zig_version" \
    "$zig_source" xz
  zig_binary="$zig_source/zig"
  [[ -x "$zig_binary" ]] || die "verified Zig archive did not provide: $zig_binary"
fi

zig_command=("$zig_binary")
if [[ "$zig_source_kind" == "official" ]]; then
  actual_zig_version="$("${zig_command[@]}" version)"
  [[ "$actual_zig_version" == "$zig_version" ]] \
    || die "wrong Zig version after extraction (expected $zig_version, got $actual_zig_version)"
fi

ghostty_prefix="$install_root/ghostty-$ghostty_version"
mkdir -p "$ghostty_prefix"
zig_build_path="$PATH"
if [[ "$zig_source_kind" == "homebrew" ]]; then
  # Apple's libtool drops Zig archive members that are not 8-byte aligned.
  # The LLVM tool installed with Homebrew Zig preserves and realigns them.
  libtool_shim_dir="$storage_root/tool-shims/homebrew-zig"
  if [[ -e "$libtool_shim_dir" ]]; then
    safe_remove_tree "$libtool_shim_dir"
  fi
  mkdir -p "$libtool_shim_dir"
  ln -s -- "$homebrew_llvm_libtool_path" "$libtool_shim_dir/libtool"
  zig_build_path="$libtool_shim_dir:$PATH"
fi
(
  cd "$ghostty_source"
  PATH="$zig_build_path" "${zig_command[@]}" build -j2 \
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
ghostty_macos_library="$xcframework/macos-arm64_x86_64/libghostty.a"
[[ -f "$ghostty_macos_library" ]] \
  || die "Ghostty XCFramework has no macOS static library: $ghostty_macos_library"
for required_symbol in _ghostty_init _ghostty_app_new; do
  nm -arch "$swift_arch" -gU "$ghostty_macos_library" 2>/dev/null \
    | awk -v symbol="$required_symbol" '$NF == symbol { found = 1 } END { exit found ? 0 : 1 }' \
    || die "Ghostty macOS static library does not export $required_symbol"
done
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
ditto "$resources" "$bundle_staging/Contents/Resources"
install -m 0644 "$ghostty_source/LICENSE" \
  "$bundle_staging/Contents/Resources/GHOSTTY-LICENSE"
codesign --force --sign - "$bundle_staging"

if [[ -e "$bundle" ]]; then
  safe_remove_tree "$bundle"
fi
mv -- "$bundle_staging" "$bundle"
safe_remove_tree "$(dirname -- "$bundle_staging")"
safe_remove_tree "$harness_stage"

echo "Created Ghostty proof harness:"
echo "  $bundle"
echo "Ghostty: $ghostty_tag"
echo "  annotated tag object: $ghostty_tag_object"
echo "  peeled source commit: $ghostty_revision"
echo "  verified release archive: $ghostty_sha"
echo "Xcode: $xcode_version ($xcode_build) from $xcode_developer_dir"
echo "Zig: $zig_version ($zig_arch toolchain on $machine_arch host)"
if [[ "$zig_source_kind" == "homebrew" ]]; then
  echo "  compatibility mode: Homebrew patched $homebrew_zig_formula bottle at $zig_binary"
fi
echo "Run manually when ready: open '$bundle'"
