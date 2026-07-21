#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd -P -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
[[ $# -eq 2 ]] || {
  echo "usage: $0 <package-private-proof-root> <package-private-output-directory>" >&2
  exit 2
}

proof_root="$1"
output="$2"
lock_file="$repo_root/packaging/prism/macos/ghostty-proof.lock"
bridge_source="$repo_root/apps/prism/native/ghostty-terminal"
tree_hasher="$repo_root/scripts/hash-prism-ghostty-tree.py"
metadata_validator="$repo_root/scripts/validate-prism-ghostty-metadata.py"
bridge_verifier="$repo_root/scripts/verify-prism-ghostty-bridge-macos.sh"
xcframework_validator="$repo_root/scripts/verify-prism-ghostty-xcframework.py"
bounded_runner="$repo_root/scripts/run-prism-bounded.py"
sdk_tree_hasher="$repo_root/scripts/hash-prism-sdk-tree.py"
sdk_validator="$repo_root/scripts/verify-prism-ghostty-sdk.py"
xcrun_shim="$repo_root/scripts/prism-ghostty-xcrun-shim.sh"

die() {
  echo "error: $*" >&2
  exit 1
}

require_command() {
  command -v "$1" >/dev/null 2>&1 || die "required command not found: $1"
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

tree_manifest_sha() {
  local root="$1"
  python3 "$tree_hasher" "$root"
}

lock_value() {
  local key="$1"
  awk -F= -v key="$key" \
    '$1 == key { sub(/^[^=]*=/, ""); print; found = 1; exit } END { if (!found) exit 1 }' \
    "$lock_file" || die "missing lock value: $key"
}

attestation_value() {
  local key="$1"
  awk -F= -v key="$key" \
    '$1 == key { print $2; found = 1; exit } END { if (!found) exit 1 }' \
    "$proof_attestation" || die "missing proof attestation value: $key"
}

require_sha256() {
  local name="$1"
  local value="$2"
  [[ ${#value} -eq 64 && "$value" != *[!0-9a-f]* ]] \
    || die "invalid SHA-256 in proof attestation: $name"
}

require_canonical_directory() {
  local root="$1"
  local path="$2"
  local label="$3"
  [[ -d "$path" && ! -L "$path" ]] || die "$label is missing or is a symlink: $path"
  local resolved
  resolved="$(realpath "$path")"
  [[ "$resolved" == "$path" ]] || die "$label contains a symlink or non-canonical component: $path"
  case "$resolved" in
    "$root"/*) ;;
    *) die "$label escapes the verified proof root: $path" ;;
  esac
}

require_canonical_file() {
  local root="$1"
  local path="$2"
  local label="$3"
  [[ -f "$path" && ! -L "$path" ]] || die "$label is missing or is a symlink: $path"
  local resolved
  resolved="$(realpath "$path")"
  [[ "$resolved" == "$path" ]] || die "$label contains a symlink or non-canonical component: $path"
  case "$resolved" in
    "$root"/*) ;;
    *) die "$label escapes the verified proof root: $path" ;;
  esac
}

verify_reviewed_inputs_unchanged() {
  [[ "$(sha256_file "$repo_root/scripts/build-prism-ghostty-bridge-macos.sh")" == \
    "$reviewed_consumer_script_sha" ]] \
    || die "Ghostty bridge builder changed during packaging"
  [[ "$(sha256_file "$lock_file")" == "$reviewed_lock_sha" ]] \
    || die "reviewed Ghostty lock changed during packaging"
  [[ "$(sha256_file "$repo_root/scripts/build-prism-ghostty-macos.sh")" == \
    "$reviewed_proof_script_sha" ]] \
    || die "Ghostty proof builder changed during packaging"
  [[ "$(sha256_file "$tree_hasher")" == "$reviewed_tree_hasher_sha" ]] \
    || die "Ghostty tree hasher changed during packaging"
  [[ "$(sha256_file "$metadata_validator")" == "$reviewed_metadata_validator_sha" ]] \
    || die "Ghostty metadata validator changed during packaging"
  [[ "$(sha256_file "$bridge_verifier")" == "$reviewed_bridge_verifier_sha" ]] \
    || die "Ghostty bridge verifier changed during packaging"
  [[ "$(sha256_file "$xcframework_validator")" == \
    "$reviewed_xcframework_validator_sha" ]] \
    || die "Ghostty XCFramework validator changed during packaging"
  [[ "$(sha256_file "$bounded_runner")" == "$reviewed_bounded_runner_sha" ]] \
    || die "bounded command runner changed during packaging"
  [[ "$(sha256_file "$sdk_tree_hasher")" == "$reviewed_sdk_tree_hasher_sha" ]] \
    || die "SDK tree hasher changed during packaging"
  [[ "$(sha256_file "$sdk_validator")" == "$reviewed_sdk_validator_sha" ]] \
    || die "SDK validator changed during packaging"
  [[ "$(sha256_file "$xcrun_shim")" == "$reviewed_xcrun_shim_sha" ]] \
    || die "xcrun shim changed during packaging"
}

verify_snapshot_hashes() {
  verify_file "$macos_library" "$library_sha"
  verify_file "$macos_header" "$header_sha"
  verify_file "$xcframework_manifest" "$xcframework_info_sha"
  verify_file "$resources/$resource_sentinel" "$sentinel_sha"
  verify_file "$ghostty_source/LICENSE" "$license_sha"
  [[ "$(tree_manifest_sha "$xcframework")" == "$xcframework_tree_sha" ]] \
    || die "Ghostty XCFramework snapshot does not match its proof attestation"
  [[ "$(tree_manifest_sha "$resources")" == "$resources_tree_sha" ]] \
    || die "Ghostty resource snapshot does not match its proof attestation"
}

verify_sealed_snapshot_unchanged() {
  [[ "$(tree_manifest_sha "$xcframework")" == "$sealed_xcframework_tree_sha" ]] \
    || die "sealed Ghostty XCFramework snapshot changed during packaging"
  [[ "$(tree_manifest_sha "$resources")" == "$sealed_resources_tree_sha" ]] \
    || die "sealed Ghostty resource snapshot changed during packaging"
  [[ "$(tree_manifest_sha "$ghostty_source")" == "$sealed_source_tree_sha" ]] \
    || die "sealed Ghostty source metadata snapshot changed during packaging"
}

[[ "$(uname -s)" == "Darwin" ]] || die "Ghostty bridge packaging requires macOS"
[[ "${MACOSX_DEPLOYMENT_TARGET:-}" == "13.0" ]] \
  || die "Ghostty bridge packaging requires MACOSX_DEPLOYMENT_TARGET=13.0"
[[ -d "$bridge_source" ]] || die "bridge source not found: $bridge_source"
[[ -f "$tree_hasher" && ! -L "$tree_hasher" ]] || die "tree hasher not found: $tree_hasher"
[[ -f "$metadata_validator" && ! -L "$metadata_validator" ]] \
  || die "metadata validator not found: $metadata_validator"
[[ -f "$bridge_verifier" && ! -L "$bridge_verifier" ]] \
  || die "bridge verifier not found: $bridge_verifier"
[[ -f "$xcframework_validator" && ! -L "$xcframework_validator" ]] \
  || die "XCFramework validator not found: $xcframework_validator"
[[ -f "$bounded_runner" && ! -L "$bounded_runner" ]] \
  || die "bounded command runner not found: $bounded_runner"
[[ -f "$sdk_tree_hasher" && ! -L "$sdk_tree_hasher" ]] \
  || die "SDK tree hasher not found: $sdk_tree_hasher"
[[ -f "$sdk_validator" && ! -L "$sdk_validator" ]] \
  || die "SDK validator not found: $sdk_validator"
[[ -f "$xcrun_shim" && ! -L "$xcrun_shim" ]] \
  || die "xcrun shim not found: $xcrun_shim"

for command in chmod codesign ditto find install install_name_tool lipo nm otool plutil python3 realpath shasum strings xcodebuild xcrun; do
  require_command "$command"
done
python3 "$metadata_validator" lock "$lock_file" \
  || die "Ghostty proof lock is malformed"
private_root="${PRISM_GHOSTTY_PRIVATE_ROOT:-}"
[[ -n "$private_root" && "$private_root" == /* && -d "$private_root" && ! -L "$private_root" ]] \
  || die "bridge packaging requires a package-owned private root"
private_root="$(cd -P -- "$private_root" && pwd)"
case "$private_root" in
  "$repo_root"/target/prism-ghostty-package.*) ;;
  *) die "private root is outside the package-owned target namespace" ;;
esac
[[ "$(stat -f %Lp "$private_root")" == "700" ]] \
  || die "package-owned private root must have mode 700"
case "$proof_root" in
  /*) ;;
  *) die "verified proof root must be absolute" ;;
esac
[[ -d "$proof_root" && ! -L "$proof_root" ]] \
  || die "verified proof root is missing or is a symlink: $proof_root"
proof_root="$(cd -P -- "$proof_root" && pwd)"
[[ "$proof_root" == "$private_root/proof" ]] \
  || die "bridge packaging rejects externally prepared proof roots"

lock_format="$(lock_value LOCK_FORMAT)"
ghostty_version="$(lock_value GHOSTTY_VERSION)"
ghostty_sha="$(lock_value GHOSTTY_SOURCE_SHA256)"
zig_version="$(lock_value ZIG_VERSION)"
macos_target="$(lock_value GHOSTTY_MACOS_TARGET)"
minimum_macos="$(lock_value MINIMUM_MACOS_VERSION)"
bridge_abi_version="$(lock_value PRISM_GHOSTTY_BRIDGE_ABI_VERSION)"
proof_attestation_relative="$(lock_value GHOSTTY_PROOF_ATTESTATION)"
xcframework_relative="$(lock_value GHOSTTY_XCFRAMEWORK_PATH)"
resources_relative="$(lock_value GHOSTTY_RESOURCES_PATH)"
resource_sentinel="$(lock_value GHOSTTY_RESOURCE_SENTINEL)"
expected_xcode_version="$(lock_value XCODE_VERSION)"
expected_xcode_build="$(lock_value XCODE_BUILD)"
expected_xcode_libtool_sha="$(lock_value XCODE_LIBTOOL_SHA256)"
expected_clt_sdk_tree_sha="$(lock_value CLT_MACOS_SDK_TREE_SHA256)"
[[ "$lock_format" == "2" ]] || die "unsupported lock format: $lock_format"
for relative_path in \
  "$proof_attestation_relative" \
  "$xcframework_relative" \
  "$resources_relative" \
  "$resource_sentinel"; do
  case "$relative_path" in
    ""|/*|../*|*/../*|*/..) die "unsafe relative path in lock file: $relative_path" ;;
  esac
done
ghostty_source="$proof_root/sources/ghostty-$ghostty_version"
xcframework="$ghostty_source/$xcframework_relative"
resources="$proof_root/install/ghostty-$ghostty_version/$resources_relative"
proof_attestation="$proof_root/$proof_attestation_relative"
macos_library="$xcframework/$macos_target/libghostty.a"
macos_header="$xcframework/$macos_target/Headers/ghostty.h"
xcframework_manifest="$xcframework/Info.plist"

require_canonical_directory "$proof_root" "$ghostty_source" "Ghostty source"
require_canonical_directory "$proof_root" "$xcframework" "Ghostty XCFramework"
require_canonical_directory "$proof_root" "$resources" "Ghostty resources"
require_canonical_file "$proof_root" "$proof_attestation" "Ghostty proof attestation"
require_canonical_file "$proof_root" "$ghostty_source/.spectrum-source-sha256" "Ghostty source marker"
require_canonical_file "$proof_root" "$xcframework_manifest" "Ghostty XCFramework manifest"
require_canonical_file "$proof_root" "$macos_library" "Ghostty macOS static library"
require_canonical_file "$proof_root" "$macos_header" "Ghostty macOS header"
require_canonical_file "$proof_root" "$resources/$resource_sentinel" "Ghostty resource sentinel"
require_canonical_file "$proof_root" "$ghostty_source/LICENSE" "Ghostty license"
if find "$xcframework" "$resources" -type l -print -quit | grep -q .; then
  die "verified Ghostty artifacts must not contain symlinks"
fi

reviewed_lock_sha="$(sha256_file "$lock_file")"
reviewed_consumer_script_sha="$(sha256_file "$repo_root/scripts/build-prism-ghostty-bridge-macos.sh")"
reviewed_proof_script_sha="$(sha256_file "$repo_root/scripts/build-prism-ghostty-macos.sh")"
reviewed_tree_hasher_sha="$(sha256_file "$tree_hasher")"
reviewed_metadata_validator_sha="$(sha256_file "$metadata_validator")"
reviewed_bridge_verifier_sha="$(sha256_file "$bridge_verifier")"
reviewed_xcframework_validator_sha="$(sha256_file "$xcframework_validator")"
reviewed_bounded_runner_sha="$(sha256_file "$bounded_runner")"
reviewed_sdk_tree_hasher_sha="$(sha256_file "$sdk_tree_hasher")"
reviewed_sdk_validator_sha="$(sha256_file "$sdk_validator")"
reviewed_xcrun_shim_sha="$(sha256_file "$xcrun_shim")"
stage="$(mktemp -d "$private_root/consumer.XXXXXX")"
cleanup() {
  if [[ -d "$stage" && ! -L "$stage" && "$(realpath "$stage")" == "$stage" ]]; then
    case "$stage" in
      "$private_root"/consumer.*)
        chmod -R u+w "$stage"
        rm -rf -- "$stage"
        ;;
    esac
  fi
}
trap cleanup EXIT

# Snapshot every proof input into controlled staging before validation, then
# build against that exact copy. No compiler path points back into the mutable
# proof tree.
snapshot_source="$stage/proof-source"
snapshot_resources="$stage/proof-resources"
mkdir -p "$snapshot_source" "$stage/package/Artifacts"
ditto "$xcframework" "$stage/package/Artifacts/GhosttyKit.xcframework"
ditto "$resources" "$snapshot_resources"
install -m 0444 "$proof_attestation" "$stage/proof.attestation"
install -m 0444 "$ghostty_source/.spectrum-source-sha256" \
  "$snapshot_source/.spectrum-source-sha256"
install -m 0444 "$ghostty_source/LICENSE" "$snapshot_source/LICENSE"

ghostty_source="$snapshot_source"
xcframework="$stage/package/Artifacts/GhosttyKit.xcframework"
resources="$snapshot_resources"
proof_attestation="$stage/proof.attestation"
macos_library="$xcframework/$macos_target/libghostty.a"
macos_header="$xcframework/$macos_target/Headers/ghostty.h"
xcframework_manifest="$xcframework/Info.plist"
require_canonical_directory "$stage" "$ghostty_source" "Ghostty source snapshot"
require_canonical_directory "$stage" "$xcframework" "Ghostty XCFramework snapshot"
require_canonical_directory "$stage" "$resources" "Ghostty resource snapshot"
require_canonical_file "$stage" "$proof_attestation" "Ghostty proof attestation snapshot"
require_canonical_file "$stage" "$ghostty_source/.spectrum-source-sha256" "Ghostty source marker snapshot"
require_canonical_file "$stage" "$xcframework_manifest" "Ghostty XCFramework manifest snapshot"
require_canonical_file "$stage" "$macos_library" "Ghostty macOS static library snapshot"
require_canonical_file "$stage" "$macos_header" "Ghostty macOS header snapshot"
require_canonical_file "$stage" "$resources/$resource_sentinel" "Ghostty resource sentinel snapshot"
require_canonical_file "$stage" "$ghostty_source/LICENSE" "Ghostty license snapshot"

python3 "$metadata_validator" attestation "$proof_attestation" \
  "ATTESTATION_FORMAT=1" \
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
  "XCODE_VERSION=$expected_xcode_version" \
  "XCODE_BUILD=$expected_xcode_build" \
  "XCODE_LIBTOOL_SHA256=$expected_xcode_libtool_sha" \
  "CLT_MACOS_SDK_TREE_SHA256=$expected_clt_sdk_tree_sha" \
  "ZIG_VERSION=$zig_version" \
  "GHOSTTY_MACOS_TARGET=$macos_target" \
  "MINIMUM_MACOS_VERSION=$minimum_macos" \
  "PRISM_GHOSTTY_BRIDGE_ABI_VERSION=$bridge_abi_version" \
  || die "proof attestation is malformed or stale"
[[ "$(attestation_value ATTESTATION_FORMAT)" == "1" ]] \
  || die "unsupported proof attestation format"
[[ "$(attestation_value LOCK_FORMAT)" == "$lock_format" ]] \
  || die "proof attestation lock format mismatch"
[[ "$(attestation_value PROOF_LOCK_SHA256)" == "$reviewed_lock_sha" ]] \
  || die "proof attestation is stale for the reviewed lock"
[[ "$(attestation_value PROOF_SCRIPT_SHA256)" == \
  "$reviewed_proof_script_sha" ]] \
  || die "proof attestation is stale for the proof builder"
[[ "$(attestation_value TREE_HASHER_SHA256)" == "$reviewed_tree_hasher_sha" ]] \
  || die "proof attestation is stale for the tree hasher"
[[ "$(attestation_value METADATA_VALIDATOR_SHA256)" == \
  "$reviewed_metadata_validator_sha" ]] \
  || die "proof attestation is stale for the metadata validator"
[[ "$(attestation_value XCFRAMEWORK_VALIDATOR_SHA256)" == \
  "$reviewed_xcframework_validator_sha" ]] \
  || die "proof attestation is stale for the XCFramework validator"
[[ "$(attestation_value BOUNDED_RUNNER_SHA256)" == "$reviewed_bounded_runner_sha" ]] \
  || die "proof attestation is stale for the bounded command runner"
[[ "$(attestation_value SDK_TREE_HASHER_SHA256)" == "$reviewed_sdk_tree_hasher_sha" ]] \
  || die "proof attestation is stale for the SDK tree hasher"
[[ "$(attestation_value SDK_VALIDATOR_SHA256)" == "$reviewed_sdk_validator_sha" ]] \
  || die "proof attestation is stale for the SDK validator"
[[ "$(attestation_value XCRUN_SHIM_SHA256)" == "$reviewed_xcrun_shim_sha" ]] \
  || die "proof attestation is stale for the xcrun shim"
[[ "$(attestation_value GHOSTTY_VERSION)" == "$ghostty_version" ]] \
  || die "proof attestation Ghostty version mismatch"
[[ "$(attestation_value GHOSTTY_SOURCE_SHA256)" == "$ghostty_sha" ]] \
  || die "proof attestation source checksum mismatch"
[[ "$(attestation_value XCODE_VERSION)" == "$expected_xcode_version" ]] \
  || die "proof attestation Xcode version mismatch"
[[ "$(attestation_value XCODE_BUILD)" == "$expected_xcode_build" ]] \
  || die "proof attestation Xcode build mismatch"
[[ "$(attestation_value XCODE_LIBTOOL_SHA256)" == "$expected_xcode_libtool_sha" ]] \
  || die "proof attestation Xcode libtool checksum mismatch"
[[ "$(attestation_value CLT_MACOS_SDK_TREE_SHA256)" == "$expected_clt_sdk_tree_sha" ]] \
  || die "proof attestation CLT macOS SDK tree checksum mismatch"
[[ "$(attestation_value ZIG_VERSION)" == "$zig_version" ]] \
  || die "proof attestation Zig version mismatch"
[[ "$(attestation_value GHOSTTY_MACOS_TARGET)" == "$macos_target" ]] \
  || die "proof attestation target mismatch"
[[ "$(attestation_value MINIMUM_MACOS_VERSION)" == "$minimum_macos" ]] \
  || die "proof attestation deployment target mismatch"
[[ "$(attestation_value PRISM_GHOSTTY_BRIDGE_ABI_VERSION)" == "$bridge_abi_version" ]] \
  || die "proof attestation bridge ABI mismatch"

library_sha="${PRISM_GHOSTTY_SEAL_LIBRARY_SHA256:-}"
header_sha="${PRISM_GHOSTTY_SEAL_HEADER_SHA256:-}"
xcframework_info_sha="${PRISM_GHOSTTY_SEAL_XCFRAMEWORK_INFO_SHA256:-}"
xcframework_tree_sha="${PRISM_GHOSTTY_SEAL_XCFRAMEWORK_TREE_SHA256:-}"
sentinel_sha="${PRISM_GHOSTTY_SEAL_SENTINEL_SHA256:-}"
resources_tree_sha="${PRISM_GHOSTTY_SEAL_RESOURCES_TREE_SHA256:-}"
license_sha="${PRISM_GHOSTTY_SEAL_LICENSE_SHA256:-}"
for digest_name in \
  library_sha \
  header_sha \
  xcframework_info_sha \
  xcframework_tree_sha \
  sentinel_sha \
  resources_tree_sha \
  license_sha; do
  require_sha256 "$digest_name" "${!digest_name}"
done
[[ "$(attestation_value GHOSTTY_MACOS_LIBRARY_SHA256)" == "$library_sha" ]] \
  || die "diagnostic attestation differs from the package-owned library seal"
[[ "$(attestation_value GHOSTTY_MACOS_HEADER_SHA256)" == "$header_sha" ]] \
  || die "diagnostic attestation differs from the package-owned header seal"
[[ "$(attestation_value GHOSTTY_XCFRAMEWORK_INFO_SHA256)" == "$xcframework_info_sha" ]] \
  || die "diagnostic attestation differs from the package-owned manifest seal"
[[ "$(attestation_value GHOSTTY_XCFRAMEWORK_TREE_SHA256)" == "$xcframework_tree_sha" ]] \
  || die "diagnostic attestation differs from the package-owned XCFramework seal"
[[ "$(attestation_value GHOSTTY_RESOURCE_SENTINEL_SHA256)" == "$sentinel_sha" ]] \
  || die "diagnostic attestation differs from the package-owned sentinel seal"
[[ "$(attestation_value GHOSTTY_RESOURCES_TREE_SHA256)" == "$resources_tree_sha" ]] \
  || die "diagnostic attestation differs from the package-owned resource seal"
[[ "$(attestation_value GHOSTTY_LICENSE_SHA256)" == "$license_sha" ]] \
  || die "diagnostic attestation differs from the package-owned license seal"
[[ -n "${DEVELOPER_DIR:-}" ]] \
  || die "set DEVELOPER_DIR to the pinned full Xcode $expected_xcode_version"
case "$DEVELOPER_DIR" in
  *.app/Contents/Developer) ;;
  *) die "DEVELOPER_DIR must name a full Xcode Developer directory" ;;
esac
xcode_output="$(xcodebuild -version)"
actual_xcode_version="$(printf '%s\n' "$xcode_output" | awk 'NR == 1 { print $2; exit }')"
actual_xcode_build="$(printf '%s\n' "$xcode_output" | awk 'NR == 2 { print $3; exit }')"
[[ "$actual_xcode_version" == "$expected_xcode_version" ]] \
  || die "Xcode version mismatch (expected $expected_xcode_version, got $actual_xcode_version)"
[[ "$actual_xcode_build" == "$expected_xcode_build" ]] \
  || die "Xcode build mismatch (expected $expected_xcode_build, got $actual_xcode_build)"
xcode_nm="$(xcrun --find nm)" || die "pinned Xcode does not provide nm"

actual_bridge_abi_version="$(awk \
  '$1 == "#define" && $2 == "PRISM_GHOSTTY_BRIDGE_ABI_VERSION" { print $3; found = 1; exit } END { if (!found) exit 1 }' \
  "$bridge_source/include/prism_ghostty_bridge.h")" \
  || die "could not read Prism Ghostty bridge ABI version"
[[ "$actual_bridge_abi_version" == "$bridge_abi_version" ]] \
  || die "Prism Ghostty bridge ABI does not match the reviewed lock"

[[ "$(sed -n '1p' "$ghostty_source/.spectrum-source-sha256")" == "$ghostty_sha" ]] \
  || die "proof source marker does not match the pinned release checksum"
verify_reviewed_inputs_unchanged
verify_snapshot_hashes
library_arches="$(lipo -archs "$macos_library")"
[[ " $library_arches " == *" arm64 "* && " $library_arches " == *" x86_64 "* ]] \
  || die "Ghostty static library must contain arm64 and x86_64 (found: $library_arches)"
for architecture in arm64 x86_64; do
  python3 "$metadata_validator" symbols \
    "$xcode_nm" \
    "$macos_library" \
    "$architecture" \
    _ghostty_init \
    _ghostty_app_new \
    _ghostty_surface_new
done
plutil -lint "$xcframework_manifest" >/dev/null \
  || die "Ghostty XCFramework manifest is invalid"
python3 "$xcframework_validator" "$xcframework_manifest" "$macos_target" \
  || die "Ghostty XCFramework manifest does not describe the reviewed macOS slice"
chmod -R a-w "$ghostty_source" "$resources" "$xcframework"
sealed_xcframework_tree_sha="$(tree_manifest_sha "$xcframework")"
sealed_resources_tree_sha="$(tree_manifest_sha "$resources")"
sealed_source_tree_sha="$(tree_manifest_sha "$ghostty_source")"
verify_sealed_snapshot_unchanged

case "$output" in
  /*) ;;
  *) die "output must be an absolute package-private path" ;;
esac
[[ "$output" == "$private_root/bridge" ]] \
  || die "bridge output must be the package-owned private bridge directory"
output_parent="$(dirname -- "$output")"
output_name="$(basename -- "$output")"
[[ "$output_name" != "." && "$output_name" != ".." ]] || die "unsafe output path: $output"
[[ -d "$output_parent" && ! -L "$output_parent" ]] \
  || die "output parent is missing or is a symlink: $output_parent"
output_parent="$(cd -P -- "$output_parent" && pwd)"
case "$output_parent" in
  "$private_root") ;;
  *) die "output must be directly below the package-owned private root" ;;
esac
output="$output_parent/$output_name"
[[ ! -L "$output" ]] || die "output must not be a symlink: $output"

cp -- "$bridge_source/Package.swift" "$stage/package/Package.swift"
cp -R -- "$bridge_source/Sources" "$stage/package/Sources"

scratch="$stage/swift-build"
xcrun swift build \
  --package-path "$stage/package" \
  --scratch-path "$scratch" \
  --configuration release
binary_directory="$(xcrun swift build \
  --package-path "$stage/package" \
  --scratch-path "$scratch" \
  --configuration release \
  --show-bin-path)"
bridge="$binary_directory/libPrismGhosttyBridge.dylib"
[[ -f "$bridge" ]] || die "Swift build did not produce $bridge"
install_name_tool -id "@rpath/libPrismGhosttyBridge.dylib" "$bridge"
machine_arch="$(uname -m)"
verify_reviewed_inputs_unchanged
verify_sealed_snapshot_unchanged
codesign --force --sign - "$bridge"
bash "$bridge_verifier" "$bridge" "$minimum_macos" "$machine_arch" \
  "$repo_root" "$proof_root" "$stage"

output_candidate="$stage/output"
mkdir -p "$output_candidate"
install -m 0755 "$bridge" "$output_candidate/libPrismGhosttyBridge.dylib"
ditto "$resources" "$output_candidate/Resources"
install -m 0644 "$ghostty_source/LICENSE" "$output_candidate/GHOSTTY-LICENSE"
[[ "$(tree_manifest_sha "$output_candidate/Resources")" == "$sealed_resources_tree_sha" ]] \
  || die "staged Ghostty resources differ from verified input"
verify_file "$output_candidate/GHOSTTY-LICENSE" "$license_sha"
bash "$bridge_verifier" "$output_candidate/libPrismGhosttyBridge.dylib" \
  "$minimum_macos" "$machine_arch" "$repo_root" "$proof_root" "$stage"
verify_reviewed_inputs_unchanged
verify_sealed_snapshot_unchanged

if [[ -e "$output" || -L "$output" ]]; then
  [[ -d "$output" && ! -L "$output" && "$(realpath "$output")" == "$output" ]] \
    || die "existing output is not a canonical real directory: $output"
  rm -rf -- "$output"
fi
mv -- "$output_candidate" "$output"

echo "Created pinned Ghostty bridge staging at $output"
