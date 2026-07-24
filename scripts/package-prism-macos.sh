#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd -P -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

ghostty_enabled=false
if [[ "${1:-}" == "--with-ghostty" && $# -eq 1 ]]; then
  ghostty_enabled=true
elif [[ $# -ne 0 ]]; then
  echo "usage: $0 [--with-ghostty]" >&2
  exit 2
fi

sha256_file() {
  shasum -a 256 "$1" | awk '{print $1}'
}

lock_value() {
  local key="$1"
  awk -F= -v key="$key" \
    '$1 == key { print $2; found = 1; exit } END { if (!found) exit 1 }' \
    "$lock_file"
}

verify_chain_sources() {
  [[ "$(sha256_file "$lock_file")" == "$chain_lock_sha" ]]
  [[ "$(sha256_file "$repo_root/scripts/package-prism-macos.sh")" == "$chain_package_sha" ]]
  [[ "$(sha256_file "$proof_builder")" == "$chain_proof_builder_sha" ]]
  [[ "$(sha256_file "$bridge_builder")" == "$chain_bridge_builder_sha" ]]
  [[ "$(sha256_file "$tree_hasher")" == "$chain_tree_hasher_sha" ]]
  [[ "$(sha256_file "$metadata_validator")" == "$chain_metadata_validator_sha" ]]
  [[ "$(sha256_file "$path_scrubber")" == "$chain_path_scrubber_sha" ]]
  [[ "$(sha256_file "$xcframework_validator")" == "$chain_xcframework_validator_sha" ]]
  [[ "$(sha256_file "$bridge_verifier")" == "$chain_bridge_verifier_sha" ]]
  [[ "$(sha256_file "$bounded_runner")" == "$chain_bounded_runner_sha" ]]
  [[ "$(sha256_file "$sdk_tree_hasher")" == "$chain_sdk_tree_hasher_sha" ]]
  [[ "$(sha256_file "$sdk_validator")" == "$chain_sdk_validator_sha" ]]
  [[ "$(sha256_file "$xcrun_shim")" == "$chain_xcrun_shim_sha" ]]
}

cleanup_private_root() {
  local exit_code=$?
  if [[ -n "${private_root:-}" && -d "$private_root" && ! -L "$private_root" \
    && "$(realpath "$private_root")" == "$private_root" ]]; then
    case "$private_root" in
      "$repo_root"/target/spectrum-ghostty-package.*)
        live_marker="$private_root/proof/.live-process-group"
        if [[ -e "$live_marker" || -L "$live_marker" ]]; then
          echo "warning: retaining Ghostty private root with live-process marker: $private_root" >&2
        else
          chmod -R u+w "$private_root" ||
            echo "warning: could not make Ghostty private root writable: $private_root" >&2
          rm -rf -- "$private_root" ||
            echo "warning: could not remove Ghostty private root: $private_root" >&2
        fi
        ;;
    esac
  fi
  return "$exit_code"
}
trap cleanup_private_root EXIT

if [[ "$ghostty_enabled" == true ]]; then
  lock_file="$repo_root/packaging/spectrum-terminal/macos/ghostty-proof.lock"
  proof_builder="$repo_root/scripts/build-spectrum-ghostty-macos.sh"
  bridge_builder="$repo_root/scripts/build-spectrum-ghostty-bridge-macos.sh"
  tree_hasher="$repo_root/scripts/hash-spectrum-ghostty-tree.py"
  metadata_validator="$repo_root/scripts/validate-spectrum-ghostty-metadata.py"
  path_scrubber="$repo_root/scripts/scrub-spectrum-binary-paths.py"
  xcframework_validator="$repo_root/scripts/verify-spectrum-ghostty-xcframework.py"
  bridge_verifier="$repo_root/scripts/verify-spectrum-ghostty-bridge-macos.sh"
  bounded_runner="$repo_root/scripts/run-spectrum-bounded.py"
  sdk_tree_hasher="$repo_root/scripts/hash-spectrum-sdk-tree.py"
  sdk_validator="$repo_root/scripts/verify-spectrum-ghostty-sdk.py"
  xcrun_shim="$repo_root/scripts/spectrum-ghostty-xcrun-shim.sh"
  python3 "$metadata_validator" lock "$lock_file"
  expected_xcode_version="$(lock_value XCODE_VERSION)"
  expected_xcode_build="$(lock_value XCODE_BUILD)"
  minimum_macos="$(lock_value MINIMUM_MACOS_VERSION)"
  bridge_abi="$(lock_value SPECTRUM_GHOSTTY_BRIDGE_ABI_VERSION)"
  [[ -n "${DEVELOPER_DIR:-}" ]] || {
    echo "Ghostty packaging requires DEVELOPER_DIR for pinned Xcode $expected_xcode_version ($expected_xcode_build)" >&2
    exit 1
  }
  xcode_output="$(xcodebuild -version)"
  [[ "$(printf '%s\n' "$xcode_output" | sed -n '1p')" == "Xcode $expected_xcode_version" ]]
  [[ "$(printf '%s\n' "$xcode_output" | sed -n '2p')" == "Build version $expected_xcode_build" ]]
  export MACOSX_DEPLOYMENT_TARGET="$minimum_macos"
  chain_lock_sha="$(sha256_file "$lock_file")"
  chain_package_sha="$(sha256_file "$repo_root/scripts/package-prism-macos.sh")"
  chain_proof_builder_sha="$(sha256_file "$proof_builder")"
  chain_bridge_builder_sha="$(sha256_file "$bridge_builder")"
  chain_tree_hasher_sha="$(sha256_file "$tree_hasher")"
  chain_metadata_validator_sha="$(sha256_file "$metadata_validator")"
  chain_path_scrubber_sha="$(sha256_file "$path_scrubber")"
  chain_xcframework_validator_sha="$(sha256_file "$xcframework_validator")"
  chain_bridge_verifier_sha="$(sha256_file "$bridge_verifier")"
  chain_bounded_runner_sha="$(sha256_file "$bounded_runner")"
  chain_sdk_tree_hasher_sha="$(sha256_file "$sdk_tree_hasher")"
  chain_sdk_validator_sha="$(sha256_file "$sdk_validator")"
  chain_xcrun_shim_sha="$(sha256_file "$xcrun_shim")"
  verify_chain_sources

  target_root="$repo_root/target"
  if [[ -e "$target_root" || -L "$target_root" ]]; then
    [[ -d "$target_root" && ! -L "$target_root" \
      && "$(realpath "$target_root")" == "$target_root" ]] || {
      echo "Ghostty packaging requires a canonical real target directory" >&2
      exit 1
    }
  else
    mkdir -- "$target_root"
  fi
  private_root="$(mktemp -d "$repo_root/target/spectrum-ghostty-package.XXXXXX")"
  [[ "$(stat -f %Lp "$private_root")" == "700" ]]
  proof_root="$private_root/proof"
  ghostty_stage="$private_root/bridge"
  bash "$proof_builder" --storage-root "$proof_root"
  verify_chain_sources

  ghostty_version="$(lock_value GHOSTTY_VERSION)"
  macos_target="$(lock_value GHOSTTY_MACOS_TARGET)"
  xcframework="$proof_root/sources/ghostty-$ghostty_version/$(lock_value GHOSTTY_XCFRAMEWORK_PATH)"
  resources="$proof_root/install/ghostty-$ghostty_version/$(lock_value GHOSTTY_RESOURCES_PATH)"
  resource_sentinel="$(lock_value GHOSTTY_RESOURCE_SENTINEL)"
  proof_source="$proof_root/sources/ghostty-$ghostty_version"
  export SPECTRUM_GHOSTTY_PRIVATE_ROOT="$private_root"
  export SPECTRUM_GHOSTTY_SEAL_LIBRARY_SHA256="$(sha256_file "$xcframework/$macos_target/libghostty.a")"
  export SPECTRUM_GHOSTTY_SEAL_HEADER_SHA256="$(sha256_file "$xcframework/$macos_target/Headers/ghostty.h")"
  export SPECTRUM_GHOSTTY_SEAL_XCFRAMEWORK_INFO_SHA256="$(sha256_file "$xcframework/Info.plist")"
  export SPECTRUM_GHOSTTY_SEAL_XCFRAMEWORK_TREE_SHA256="$(python3 "$tree_hasher" "$xcframework")"
  export SPECTRUM_GHOSTTY_SEAL_SENTINEL_SHA256="$(sha256_file "$resources/$resource_sentinel")"
  export SPECTRUM_GHOSTTY_SEAL_RESOURCES_TREE_SHA256="$(python3 "$tree_hasher" "$resources")"
  export SPECTRUM_GHOSTTY_SEAL_LICENSE_SHA256="$(sha256_file "$proof_source/LICENSE")"
  verify_chain_sources
  bash "$bridge_builder" "$proof_root" "$ghostty_stage"
  verify_chain_sources

  packaged_bridge_sha="$(sha256_file "$ghostty_stage/libSpectrumGhosttyBridge.dylib")"
  packaged_resources_sha="$(python3 "$tree_hasher" "$ghostty_stage/Resources")"
  packaged_license_sha="$(sha256_file "$ghostty_stage/GHOSTTY-LICENSE")"
fi
if [[ "$ghostty_enabled" == true ]]; then
  cargo build --release --locked -p prism --bins --features ghostty-terminal
else
  cargo build --release --locked -p prism --bins
fi

if [[ "$ghostty_enabled" == true ]]; then
  verify_chain_sources
  [[ "$(sha256_file "$ghostty_stage/libSpectrumGhosttyBridge.dylib")" == "$packaged_bridge_sha" ]]
  [[ "$(python3 "$tree_hasher" "$ghostty_stage/Resources")" == "$packaged_resources_sha" ]]
  [[ "$(sha256_file "$ghostty_stage/GHOSTTY-LICENSE")" == "$packaged_license_sha" ]]
fi

bundle="$repo_root/target/dist/Prism.app"
if [[ -e "$bundle" || -L "$bundle" ]]; then
  [[ -d "$bundle" && ! -L "$bundle" && "$(realpath "$bundle")" == "$bundle" ]] || {
    echo "refusing to replace unsafe bundle path: $bundle" >&2
    exit 1
  }
  chmod -R u+w "$bundle"
  rm -rf -- "$bundle"
fi
mkdir -p "$bundle/Contents/MacOS" "$bundle/Contents/Resources"
install -m 0755 "$repo_root/target/release/prism-gui" "$bundle/Contents/MacOS/prism-gui"
install -m 0755 "$repo_root/target/release/prism" "$bundle/Contents/MacOS/prism"
install -m 0644 "$repo_root/packaging/prism/macos/Info.plist" "$bundle/Contents/Info.plist"
"$repo_root/scripts/package-macos-icon.sh" \
  "$repo_root/assets/branding/Prism.icon" \
  "$bundle/Contents/Resources/Prism.icns"
install -m 0644 "$repo_root/LICENSE" "$bundle/Contents/Resources/LICENSE"
install -m 0644 "$repo_root/THIRD_PARTY.md" "$bundle/Contents/Resources/THIRD_PARTY.md"
install -m 0644 \
  "$repo_root/packaging/prism/licenses/UBUNTU-FONT-LICENCE-1.0.txt" \
  "$bundle/Contents/Resources/UBUNTU-FONT-LICENCE-1.0.txt"

if [[ "$ghostty_enabled" == true ]]; then
  mkdir -p "$bundle/Contents/Frameworks"
  install -m 0755 "$ghostty_stage/libSpectrumGhosttyBridge.dylib" \
    "$bundle/Contents/Frameworks/libSpectrumGhosttyBridge.dylib"
  ditto "$ghostty_stage/Resources" "$bundle/Contents/Resources"
  install -m 0644 "$ghostty_stage/GHOSTTY-LICENSE" \
    "$bundle/Contents/Resources/GHOSTTY-LICENSE"
  [[ "$(sha256_file "$bundle/Contents/Frameworks/libSpectrumGhosttyBridge.dylib")" == "$packaged_bridge_sha" ]]
  [[ "$(sha256_file "$bundle/Contents/Resources/GHOSTTY-LICENSE")" == "$packaged_license_sha" ]]
  python3 "$tree_hasher" --verify-overlay \
    "$ghostty_stage/Resources" "$bundle/Contents/Resources"
  plutil -replace LSMinimumSystemVersion -string "$minimum_macos" "$bundle/Contents/Info.plist"
  plutil -insert SpectrumTerminalBackend -string Ghostty "$bundle/Contents/Info.plist"
  plutil -insert SpectrumGhosttyBridgeABI -integer "$bridge_abi" "$bundle/Contents/Info.plist"
  [[ "$(plutil -extract LSMinimumSystemVersion raw -o - "$bundle/Contents/Info.plist")" == "$minimum_macos" ]]
  [[ "$(plutil -extract SpectrumTerminalBackend raw -o - "$bundle/Contents/Info.plist")" == "Ghostty" ]]
  for binary in "$bundle/Contents/MacOS/prism-gui" "$bundle/Contents/MacOS/prism"; do
    otool -l "$binary" | awk -v expected="$minimum_macos" '
      $1 == "cmd" && $2 == "LC_BUILD_VERSION" { in_build = 1; next }
      in_build && $1 == "minos" { found = ($2 == expected); exit }
      END { exit found ? 0 : 1 }
    '
  done
  verify_chain_sources
fi

cli="$repo_root/target/dist/prism-macos"
install -m 0755 "$repo_root/target/release/prism" "$cli"

codesign --force --deep --sign - "$bundle"
codesign --verify --deep --strict "$bundle"
if [[ "$ghostty_enabled" == true ]]; then
  verify_chain_sources
fi
echo "Created $bundle and $cli"
