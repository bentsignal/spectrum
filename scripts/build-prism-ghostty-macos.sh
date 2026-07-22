#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd -P -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
lock_file="$repo_root/packaging/prism/macos/ghostty-proof.lock"
harness_source="$repo_root/apps/prism/native/ghostty-proof"
target_root="$repo_root/target"
storage_root="$target_root/ghostty-proof"
if [[ $# -ne 0 ]]; then
  [[ $# -eq 2 && "$1" == "--storage-root" ]] || {
    echo "usage: $0 [--storage-root <private-target-directory>]" >&2
    exit 2
  }
  storage_root="$2"
fi
case "$storage_root" in
  "$target_root"/*) ;;
  *)
    echo "error: proof storage must be an absolute descendant of $target_root" >&2
    exit 1
    ;;
esac
tree_hasher="$repo_root/scripts/hash-prism-ghostty-tree.py"
metadata_validator="$repo_root/scripts/validate-prism-ghostty-metadata.py"
xcframework_validator="$repo_root/scripts/verify-prism-ghostty-xcframework.py"
bounded_runner="$repo_root/scripts/run-prism-bounded.py"
sdk_tree_hasher="$repo_root/scripts/hash-prism-sdk-tree.py"
sdk_validator="$repo_root/scripts/verify-prism-ghostty-sdk.py"
xcrun_shim_source="$repo_root/scripts/prism-ghostty-xcrun-shim.sh"
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

tree_manifest_sha() {
  local root="$1"
  python3 "$tree_hasher" "$root"
}

run_bounded() {
  local seconds="$1"
  shift
  python3 "$bounded_runner" "$seconds" "$storage_root/.live-process-group" -- "$@"
}

verify_reviewed_inputs_unchanged() {
  [[ "$(sha256_file "$lock_file")" == "$reviewed_lock_sha" ]] \
    || die "reviewed Ghostty lock changed during proof build"
  [[ "$(sha256_file "$repo_root/scripts/build-prism-ghostty-macos.sh")" == \
    "$reviewed_proof_script_sha" ]] \
    || die "Ghostty proof builder changed during proof build"
  [[ "$(sha256_file "$tree_hasher")" == "$reviewed_tree_hasher_sha" ]] \
    || die "Ghostty tree hasher changed during proof build"
  [[ "$(sha256_file "$metadata_validator")" == "$reviewed_metadata_validator_sha" ]] \
    || die "Ghostty metadata validator changed during proof build"
  [[ "$(sha256_file "$xcframework_validator")" == "$reviewed_xcframework_validator_sha" ]] \
    || die "Ghostty XCFramework validator changed during proof build"
  [[ "$(sha256_file "$bounded_runner")" == "$reviewed_bounded_runner_sha" ]] \
    || die "bounded command runner changed during proof build"
  [[ "$(sha256_file "$sdk_tree_hasher")" == "$reviewed_sdk_tree_hasher_sha" ]] \
    || die "SDK tree hasher changed during proof build"
  [[ "$(sha256_file "$sdk_validator")" == "$reviewed_sdk_validator_sha" ]] \
    || die "SDK validator changed during proof build"
  [[ "$(sha256_file "$xcrun_shim_source")" == "$reviewed_xcrun_shim_sha" ]] \
    || die "xcrun shim source changed during proof build"
  verify_pinned_archive_tool
  verify_pinned_sdk_and_shim
}

verify_pinned_archive_tool() {
  [[ "$(brew --prefix "$homebrew_llvm_formula" 2>/dev/null || true)" == \
    "$homebrew_llvm_prefix" ]] \
    || die "Homebrew $homebrew_llvm_formula is not installed at the reviewed prefix"
  [[ "$(realpath "$homebrew_llvm_prefix")" == "$homebrew_llvm_keg" ]] \
    || die "Homebrew $homebrew_llvm_formula changed its reviewed keg"
  [[ -f "$homebrew_llvm_receipt" && ! -L "$homebrew_llvm_receipt" ]] \
    || die "Homebrew $homebrew_llvm_formula has no real bottle receipt"
  [[ "$(plutil -extract poured_from_bottle raw -o - "$homebrew_llvm_receipt" 2>/dev/null || true)" == \
    "true" ]] || die "Homebrew $homebrew_llvm_formula was not poured from a bottle"
  [[ "$($llvm_libtool --version 2>/dev/null | awk \
    'NR == 1 && $1 == "Homebrew" && $2 == "LLVM" && $3 == "version" { print $4; exit }')" == \
    "$homebrew_llvm_version" ]] || die "reviewed LLVM libtool version changed"
  [[ "$(lipo -archs "$llvm_libtool" 2>/dev/null || true)" == "$homebrew_llvm_arch" ]] \
    || die "reviewed LLVM libtool architecture changed"
  codesign --verify "$llvm_libtool" \
    || die "reviewed LLVM libtool has an invalid code signature"
  python3 "$metadata_validator" tool \
    "$homebrew_llvm_keg" \
    "$homebrew_llvm_libtool_relative" \
    "$llvm_libtool_sha" \
    "$libtool_shim" \
    || die "pinned LLVM libtool or its private shim changed during proof build"
}

verify_pinned_sdk_and_shim() {
  [[ -f "$xcrun_shim" && ! -L "$xcrun_shim" \
    && "$(sha256_file "$xcrun_shim")" == "$reviewed_xcrun_shim_sha" ]] \
    || die "package-private xcrun shim changed during proof build"
  python3 "$sdk_validator" \
    "$clt_macos_sdk_root" \
    "$clt_macos_sdk_name" \
    "$clt_macos_sdk_version" \
    "$clt_macos_sdk_tree_sha" \
    "$clt_macos_sdk_settings_sha" \
    "$clt_macos_sdk_libsystem_sha" \
    "$clt_macos_sdk_libcxx_sha" \
    "$sdk_tree_hasher" \
    "$clt_macos_sdk_uid" \
    "$clt_macos_sdk_gid" \
    prism-sdk-v1 \
    || die "pinned CLT macOS SDK changed during proof build"
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
  [[ ! -L "$destination" ]] || die "download cache entry is a symlink: $destination"
  if [[ -f "$destination" ]]; then
    verify_file "$destination" "$expected"
    return
  fi

  local temporary
  temporary="$(mktemp "$destination.part.XXXXXX")"
  curl --fail --location --proto '=https' --tlsv1.2 --output "$temporary" "$url"
  verify_file "$temporary" "$expected"
  mv -- "$temporary" "$destination"
}

safe_remove_tree() {
  local target="$1"
  [[ -n "${canonical_storage_root:-}" ]] || die "proof storage is not initialized"
  [[ -e "$target" && ! -L "$target" ]] || die "refusing to remove missing or symlinked path: $target"
  local resolved
  resolved="$(realpath "$target")" || die "could not canonicalize removal target: $target"
  [[ "$resolved" == "$target" ]] || die "refusing to remove non-canonical path: $target"
  case "$resolved" in
    "$canonical_storage_root"/*) rm -rf -- "$resolved" ;;
    *) die "refusing to remove path outside proof storage: $target" ;;
  esac
}

ensure_storage_directory() {
  local directory="$1"
  case "$directory" in
    "$canonical_storage_root"/*) ;;
    *) die "refusing to create directory outside proof storage: $directory" ;;
  esac
  if [[ -e "$directory" || -L "$directory" ]]; then
    [[ -d "$directory" && ! -L "$directory" ]] \
      || die "proof storage path is not a real directory: $directory"
  else
    mkdir -- "$directory"
  fi
  [[ "$(realpath "$directory")" == "$directory" ]] \
    || die "proof storage path has a symlinked ancestor: $directory"
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
[[ -f "$tree_hasher" && ! -L "$tree_hasher" ]] || die "tree hasher not found: $tree_hasher"
[[ -f "$metadata_validator" && ! -L "$metadata_validator" ]] \
  || die "metadata validator not found: $metadata_validator"
[[ -f "$xcframework_validator" && ! -L "$xcframework_validator" ]] \
  || die "XCFramework validator not found: $xcframework_validator"
[[ -f "$bounded_runner" && ! -L "$bounded_runner" ]] \
  || die "bounded command runner not found: $bounded_runner"
[[ -f "$sdk_tree_hasher" && ! -L "$sdk_tree_hasher" ]] \
  || die "SDK tree hasher not found: $sdk_tree_hasher"
[[ -f "$sdk_validator" && ! -L "$sdk_validator" ]] \
  || die "SDK validator not found: $sdk_validator"
[[ -f "$xcrun_shim_source" && ! -L "$xcrun_shim_source" ]] \
  || die "xcrun shim not found: $xcrun_shim_source"
[[ "$(uname -s)" == "Darwin" ]] || die "GhosttyKit proof builds require macOS"

require_command awk
require_command brew
require_command codesign
require_command curl
require_command ditto
require_command ln
require_command lipo
require_command mktemp
require_command nm
require_command plutil
require_command python3
require_command realpath
require_command shasum
require_command sw_vers
require_command tar
require_command xcode-select
require_command xcodebuild
require_command xcrun

python3 "$metadata_validator" lock "$lock_file" \
  || die "Ghostty proof lock is malformed"
reviewed_lock_sha="$(sha256_file "$lock_file")"
reviewed_proof_script_sha="$(sha256_file "$repo_root/scripts/build-prism-ghostty-macos.sh")"
reviewed_tree_hasher_sha="$(sha256_file "$tree_hasher")"
reviewed_metadata_validator_sha="$(sha256_file "$metadata_validator")"
reviewed_xcframework_validator_sha="$(sha256_file "$xcframework_validator")"
reviewed_bounded_runner_sha="$(sha256_file "$bounded_runner")"
reviewed_sdk_tree_hasher_sha="$(sha256_file "$sdk_tree_hasher")"
reviewed_sdk_validator_sha="$(sha256_file "$sdk_validator")"
reviewed_xcrun_shim_sha="$(sha256_file "$xcrun_shim_source")"
if [[ -e "$target_root" || -L "$target_root" ]]; then
  [[ -d "$target_root" && ! -L "$target_root" ]] \
    || die "repository target directory is not a real directory: $target_root"
else
  mkdir -- "$target_root"
fi
canonical_target_root="$(realpath "$target_root")"
[[ "$canonical_target_root" == "$target_root" ]] \
  || die "repository target directory has a symlinked ancestor: $target_root"
if [[ -e "$storage_root" || -L "$storage_root" ]]; then
  [[ -d "$storage_root" && ! -L "$storage_root" ]] \
    || die "proof storage root is not a real directory: $storage_root"
else
  mkdir -- "$storage_root"
fi
canonical_storage_root="$(realpath "$storage_root")"
[[ "$canonical_storage_root" == "$storage_root" ]] \
  || die "proof storage root has a symlinked ancestor: $storage_root"

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
clt_macos_sdk_root="$(lock_value CLT_MACOS_SDK_PATH)"
clt_macos_sdk_name="$(lock_value CLT_MACOS_SDK_CANONICAL_NAME)"
clt_macos_sdk_version="$(lock_value CLT_MACOS_SDK_VERSION)"
clt_macos_sdk_uid="$(lock_value CLT_MACOS_SDK_UID)"
clt_macos_sdk_gid="$(lock_value CLT_MACOS_SDK_GID)"
clt_macos_sdk_tree_sha="$(lock_value CLT_MACOS_SDK_TREE_SHA256)"
clt_macos_sdk_settings_sha="$(lock_value CLT_MACOS_SDK_SETTINGS_SHA256)"
clt_macos_sdk_libsystem_sha="$(lock_value CLT_MACOS_SDK_LIBSYSTEM_SHA256)"
clt_macos_sdk_libcxx_sha="$(lock_value CLT_MACOS_SDK_LIBCXX_SHA256)"
expected_xcode_version="$(lock_value XCODE_VERSION)"
expected_xcode_build="$(lock_value XCODE_BUILD)"
homebrew_llvm_formula="$(lock_value HOMEBREW_LLVM_FORMULA)"
homebrew_llvm_version="$(lock_value HOMEBREW_LLVM_VERSION)"
homebrew_llvm_arch="$(lock_value HOMEBREW_LLVM_ARCH)"
homebrew_llvm_prefix="$(lock_value HOMEBREW_LLVM_PREFIX)"
homebrew_llvm_libtool_relative="$(lock_value HOMEBREW_LLVM_LIBTOOL_RELATIVE)"
llvm_libtool_sha="$(lock_value LLVM_LIBTOOL_SHA256)"
minimum_macos="$(lock_value MINIMUM_MACOS_VERSION)"
macos_target="$(lock_value GHOSTTY_MACOS_TARGET)"
bridge_abi_version="$(lock_value PRISM_GHOSTTY_BRIDGE_ABI_VERSION)"
proof_attestation_relative="$(lock_value GHOSTTY_PROOF_ATTESTATION)"
xcframework_relative="$(lock_value GHOSTTY_XCFRAMEWORK_PATH)"
resources_relative="$(lock_value GHOSTTY_RESOURCES_PATH)"
resource_sentinel="$(lock_value GHOSTTY_RESOURCE_SENTINEL)"

[[ "$lock_format" == "2" ]] || die "unsupported lock format: $lock_format"
[[ "$ghostty_version" == "1.3.1" ]] || die "this proof expects Ghostty 1.3.1"
[[ "$ghostty_tag" == "v$ghostty_version" ]] || die "Ghostty tag/version mismatch in lock file"
[[ "$ghostty_tag_object" =~ ^[0-9a-f]{40}$ ]] \
  || die "invalid Ghostty annotated tag object in lock file"
[[ "$ghostty_revision" =~ ^[0-9a-f]{40}$ ]] \
  || die "invalid Ghostty peeled source revision in lock file"
[[ "$ghostty_tag_object" != "$ghostty_revision" ]] \
  || die "Ghostty annotated tag object and peeled source revision must be distinct"
[[ "$zig_version" == "0.15.2" ]] || die "this proof expects upstream's exact Zig 0.15.2 toolchain"
[[ "$xcode_version" == "$expected_xcode_version" ]] \
  || die "Xcode version mismatch (expected $expected_xcode_version, got $xcode_version)"
[[ "$xcode_build" == "$expected_xcode_build" ]] \
  || die "Xcode build mismatch (expected $expected_xcode_build, got $xcode_build)"
[[ "$minimum_macos" == "13.0" ]] || die "this proof expects Ghostty's macOS 13.0 deployment target"
[[ "$macos_target" == "macos-arm64_x86_64" ]] \
  || die "this proof expects Ghostty's universal macOS target"
[[ "$bridge_abi_version" =~ ^[1-9][0-9]*$ ]] \
  || die "invalid Prism Ghostty bridge ABI version: $bridge_abi_version"
[[ "$homebrew_llvm_formula" == "llvm@20" \
  && "$homebrew_llvm_version" == "20.1.8" \
  && "$homebrew_llvm_arch" == "arm64" \
  && "$homebrew_llvm_prefix" == "/opt/homebrew/opt/llvm@20" \
  && "$homebrew_llvm_libtool_relative" == "bin/llvm-libtool-darwin" ]] \
  || die "unsupported LLVM archive-tool contract"
actual_bridge_abi_version="$(awk \
  '$1 == "#define" && $2 == "PRISM_GHOSTTY_BRIDGE_ABI_VERSION" { print $3; found = 1; exit } END { if (!found) exit 1 }' \
  "$repo_root/apps/prism/native/ghostty-terminal/include/prism_ghostty_bridge.h")" \
  || die "could not read Prism Ghostty bridge ABI version"
[[ "$actual_bridge_abi_version" == "$bridge_abi_version" ]] \
  || die "Prism Ghostty bridge ABI does not match the reviewed lock"
for relative_path in \
  "$xcframework_relative" \
  "$resources_relative" \
  "$resource_sentinel" \
  "$proof_attestation_relative"; do
  case "$relative_path" in
    ""|/*|../*|*/../*|*/..) die "unsafe relative path in lock file: $relative_path" ;;
  esac
done

machine_arch="$(uname -m)"
case "$machine_arch" in
  arm64)
    swift_arch="arm64"
    zig_arch="aarch64"
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

for directory in \
  "$downloads_dir" \
  "$sources_dir" \
  "$toolchains_dir" \
  "$install_root" \
  "$scratch_root" \
  "$zig_global_cache" \
  "$dist_root"; do
  ensure_storage_directory "$directory"
done
xcode_developer_dir="$(cd -P -- "$xcode_developer_dir" && pwd)"
canonical_xcode_developer_dir="$xcode_developer_dir"
xcode_nm="$(DEVELOPER_DIR="$canonical_xcode_developer_dir" xcrun --find nm)" \
  || die "selected Xcode does not provide nm"
homebrew_llvm_keg="$(realpath "$homebrew_llvm_prefix")" \
  || die "could not resolve reviewed Homebrew LLVM prefix"
case "$homebrew_llvm_keg" in
  /opt/homebrew/Cellar/llvm@20/*) ;;
  *) die "Homebrew LLVM resolves outside its reviewed Cellar" ;;
esac
homebrew_llvm_receipt="$homebrew_llvm_keg/INSTALL_RECEIPT.json"
llvm_libtool="$homebrew_llvm_keg/$homebrew_llvm_libtool_relative"
tool_shims_root="$storage_root/tool-shims"
if [[ -e "$tool_shims_root" || -L "$tool_shims_root" ]]; then
  safe_remove_tree "$tool_shims_root"
fi
ensure_storage_directory "$tool_shims_root"
libtool_shim_dir="$tool_shims_root/xcode"
ensure_storage_directory "$libtool_shim_dir"
libtool_shim="$libtool_shim_dir/libtool"
ln -s -- "$llvm_libtool" "$libtool_shim"
xcrun_shim="$libtool_shim_dir/xcrun"
install -m 0555 "$xcrun_shim_source" "$xcrun_shim"
verify_pinned_archive_tool
verify_pinned_sdk_and_shim
proof_attestation="$storage_root/$proof_attestation_relative"
rm -f -- "$proof_attestation"

ghostty_archive="$downloads_dir/ghostty-$ghostty_version.tar.gz"
download_verified "$ghostty_url" "$ghostty_sha" "$ghostty_archive"

ghostty_source="$sources_dir/ghostty-$ghostty_version"
if [[ -e "$ghostty_source" || -L "$ghostty_source" ]]; then
  safe_remove_tree "$ghostty_source"
fi
extract_once "$ghostty_archive" "$ghostty_sha" "ghostty-$ghostty_version" \
  "$ghostty_source" gz
zig_archive="$downloads_dir/zig-$zig_arch-macos-$zig_version.tar.xz"
download_verified "$zig_url" "$zig_sha" "$zig_archive"
zig_source="$toolchains_dir/zig-$zig_arch-macos-$zig_version"
if [[ -e "$zig_source" || -L "$zig_source" ]]; then
  safe_remove_tree "$zig_source"
fi
extract_once "$zig_archive" "$zig_sha" "zig-$zig_arch-macos-$zig_version" \
  "$zig_source" xz
zig_binary="$zig_source/zig"
[[ -x "$zig_binary" ]] || die "verified Zig archive did not provide: $zig_binary"

zig_command=("$zig_binary")
actual_zig_version="$(run_bounded 120 "${zig_command[@]}" version)"
[[ "$actual_zig_version" == "$zig_version" ]] \
  || die "wrong Zig version after extraction (expected $zig_version, got $actual_zig_version)"

ghostty_prefix="$install_root/ghostty-$ghostty_version"
if [[ -e "$ghostty_prefix" || -L "$ghostty_prefix" ]]; then
  safe_remove_tree "$ghostty_prefix"
fi
ensure_storage_directory "$ghostty_prefix"
(
  cd "$ghostty_source"
  verify_pinned_archive_tool
  verify_pinned_sdk_and_shim
  run_bounded 3600 env \
    PATH="$libtool_shim_dir:$PATH" \
    PRISM_GHOSTTY_MACOS_SDK_ROOT="$clt_macos_sdk_root" \
    "${zig_command[@]}" build -j2 \
    --prefix "$ghostty_prefix" \
    --cache-dir "$scratch_root/ghostty-zig-cache" \
    --global-cache-dir "$zig_global_cache" \
    -Doptimize=ReleaseFast \
    -Demit-xcframework=true \
    -Demit-macos-app=false
)
verify_pinned_archive_tool
verify_pinned_sdk_and_shim

xcframework="$ghostty_source/$xcframework_relative"
resources="$ghostty_prefix/$resources_relative"
[[ -d "$xcframework" ]] || die "Ghostty build did not produce expected XCFramework: $xcframework"
[[ -f "$xcframework/Info.plist" ]] || die "Ghostty XCFramework has no Info.plist: $xcframework"
ghostty_macos_root="$xcframework/$macos_target"
ghostty_macos_library="$ghostty_macos_root/libghostty.a"
[[ -f "$ghostty_macos_library" ]] \
  || die "Ghostty XCFramework has no macOS static library: $ghostty_macos_library"
library_arches="$(lipo -archs "$ghostty_macos_library")"
[[ " $library_arches " == *" arm64 "* && " $library_arches " == *" x86_64 "* ]] \
  || die "Ghostty static library must contain arm64 and x86_64 (found: $library_arches)"
for architecture in arm64 x86_64; do
  python3 "$metadata_validator" symbols \
    "$xcode_nm" \
    "$ghostty_macos_library" \
    "$architecture" \
    _ghostty_init \
    _ghostty_app_new \
    _ghostty_surface_new
done
[[ -f "$resources/$resource_sentinel" ]] \
  || die "Ghostty resources are incomplete; missing sentinel: $resources/$resource_sentinel"
ghostty_header="$ghostty_macos_root/Headers/ghostty.h"
[[ -f "$ghostty_header" ]] || die "Ghostty XCFramework contains no macOS ghostty.h"
[[ -f "$ghostty_source/LICENSE" ]] || die "Ghostty source contains no license"
if find "$xcframework" "$resources" -type l -print -quit | grep -q .; then
  die "Ghostty proof artifacts must not contain symlinks"
fi
plutil -lint "$xcframework/Info.plist" >/dev/null \
  || die "Ghostty XCFramework manifest is invalid"
python3 "$xcframework_validator" "$xcframework/Info.plist" "$macos_target" \
  || die "Ghostty XCFramework manifest does not describe the reviewed macOS slice"
proof_library_sha="$(sha256_file "$ghostty_macos_library")"
proof_header_sha="$(sha256_file "$ghostty_header")"
proof_xcframework_info_sha="$(sha256_file "$xcframework/Info.plist")"
proof_xcframework_tree_sha="$(tree_manifest_sha "$xcframework")"
proof_sentinel_sha="$(sha256_file "$resources/$resource_sentinel")"
proof_resources_tree_sha="$(tree_manifest_sha "$resources")"
proof_license_sha="$(sha256_file "$ghostty_source/LICENSE")"

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
codesign --verify --deep --strict "$bundle_staging"

if [[ -e "$bundle" || -L "$bundle" ]]; then
  safe_remove_tree "$bundle"
fi
mv -- "$bundle_staging" "$bundle"
safe_remove_tree "$(dirname -- "$bundle_staging")"
safe_remove_tree "$harness_stage"

verify_reviewed_inputs_unchanged
[[ "$(tree_manifest_sha "$xcframework")" == "$proof_xcframework_tree_sha" ]] \
  || die "Ghostty XCFramework changed during proof build"
[[ "$(tree_manifest_sha "$resources")" == "$proof_resources_tree_sha" ]] \
  || die "Ghostty resources changed during proof build"
verify_file "$ghostty_source/LICENSE" "$proof_license_sha"
[[ "$(sed -n '1p' "$ghostty_source/.spectrum-source-sha256")" == "$ghostty_sha" ]] \
  || die "Ghostty source marker changed during proof build"

# Generated archives and XCFramework metadata are not byte-reproducible across
# otherwise identical Zig builds. Record diagnostic hashes from this completed,
# source- and toolchain-verified proof run. Production packaging establishes
# authority by sealing its own fresh private proof, not by trusting this file.
attestation_staging="$(mktemp "$storage_root/attestation.XXXXXX")"
cat >"$attestation_staging" <<EOF
ATTESTATION_FORMAT=1
LOCK_FORMAT=$lock_format
PROOF_LOCK_SHA256=$reviewed_lock_sha
PROOF_SCRIPT_SHA256=$reviewed_proof_script_sha
TREE_HASHER_SHA256=$reviewed_tree_hasher_sha
METADATA_VALIDATOR_SHA256=$reviewed_metadata_validator_sha
XCFRAMEWORK_VALIDATOR_SHA256=$reviewed_xcframework_validator_sha
BOUNDED_RUNNER_SHA256=$reviewed_bounded_runner_sha
SDK_TREE_HASHER_SHA256=$reviewed_sdk_tree_hasher_sha
SDK_VALIDATOR_SHA256=$reviewed_sdk_validator_sha
XCRUN_SHIM_SHA256=$reviewed_xcrun_shim_sha
GHOSTTY_VERSION=$ghostty_version
GHOSTTY_SOURCE_SHA256=$ghostty_sha
XCODE_VERSION=$xcode_version
XCODE_BUILD=$xcode_build
LLVM_LIBTOOL_SHA256=$llvm_libtool_sha
CLT_MACOS_SDK_TREE_SHA256=$clt_macos_sdk_tree_sha
ZIG_VERSION=$actual_zig_version
GHOSTTY_MACOS_TARGET=$macos_target
MINIMUM_MACOS_VERSION=$minimum_macos
PRISM_GHOSTTY_BRIDGE_ABI_VERSION=$bridge_abi_version
GHOSTTY_MACOS_LIBRARY_SHA256=$proof_library_sha
GHOSTTY_MACOS_HEADER_SHA256=$proof_header_sha
GHOSTTY_XCFRAMEWORK_INFO_SHA256=$proof_xcframework_info_sha
GHOSTTY_XCFRAMEWORK_TREE_SHA256=$proof_xcframework_tree_sha
GHOSTTY_RESOURCE_SENTINEL_SHA256=$proof_sentinel_sha
GHOSTTY_RESOURCES_TREE_SHA256=$proof_resources_tree_sha
GHOSTTY_LICENSE_SHA256=$proof_license_sha
EOF
python3 "$metadata_validator" attestation "$attestation_staging" \
  "LOCK_FORMAT=$lock_format" \
  "PROOF_LOCK_SHA256=$reviewed_lock_sha" \
  "PROOF_SCRIPT_SHA256=$reviewed_proof_script_sha" \
  "TREE_HASHER_SHA256=$reviewed_tree_hasher_sha" \
  "METADATA_VALIDATOR_SHA256=$reviewed_metadata_validator_sha" \
  "XCFRAMEWORK_VALIDATOR_SHA256=$reviewed_xcframework_validator_sha" \
  "BOUNDED_RUNNER_SHA256=$reviewed_bounded_runner_sha" \
  "SDK_TREE_HASHER_SHA256=$reviewed_sdk_tree_hasher_sha" \
  "SDK_VALIDATOR_SHA256=$reviewed_sdk_validator_sha" \
  "XCRUN_SHIM_SHA256=$reviewed_xcrun_shim_sha" \
  "GHOSTTY_VERSION=$ghostty_version" \
  "GHOSTTY_SOURCE_SHA256=$ghostty_sha" \
  "XCODE_VERSION=$xcode_version" \
  "XCODE_BUILD=$xcode_build" \
  "LLVM_LIBTOOL_SHA256=$llvm_libtool_sha" \
  "CLT_MACOS_SDK_TREE_SHA256=$clt_macos_sdk_tree_sha" \
  "ZIG_VERSION=$actual_zig_version" \
  "GHOSTTY_MACOS_TARGET=$macos_target" \
  "MINIMUM_MACOS_VERSION=$minimum_macos" \
  "PRISM_GHOSTTY_BRIDGE_ABI_VERSION=$bridge_abi_version"
mv -- "$attestation_staging" "$proof_attestation"

echo "Created Ghostty proof harness:"
echo "  $bundle"
echo "Proof attestation:"
echo "  $proof_attestation"
echo "Ghostty: $ghostty_tag"
echo "  annotated tag object: $ghostty_tag_object"
echo "  peeled source commit: $ghostty_revision"
echo "  verified release archive: $ghostty_sha"
echo "Xcode: $xcode_version ($xcode_build) from $xcode_developer_dir"
echo "  LLVM libtool: $homebrew_llvm_version ($llvm_libtool_sha) at $llvm_libtool"
echo "Zig: $zig_version ($zig_arch toolchain on $machine_arch host)"
echo "  pinned macOS SDK: $clt_macos_sdk_name at $clt_macos_sdk_root"
echo "Run manually when ready: open '$bundle'"
