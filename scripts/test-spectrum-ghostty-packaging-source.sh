#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd -P -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
metadata_validator="$repo_root/scripts/validate-spectrum-ghostty-metadata.py"
path_scrubber="$repo_root/scripts/scrub-spectrum-binary-paths.py"
tree_hasher="$repo_root/scripts/hash-spectrum-ghostty-tree.py"
bridge_verifier="$repo_root/scripts/verify-spectrum-ghostty-bridge-macos.sh"
xcframework_validator="$repo_root/scripts/verify-spectrum-ghostty-xcframework.py"
bounded_runner="$repo_root/scripts/run-spectrum-bounded.py"
sdk_tree_hasher="$repo_root/scripts/hash-spectrum-sdk-tree.py"
sdk_validator="$repo_root/scripts/verify-spectrum-ghostty-sdk.py"
xcrun_shim="$repo_root/scripts/spectrum-ghostty-xcrun-shim.sh"
package_script="$repo_root/scripts/package-prism-macos.sh"
lumen_package_script="$repo_root/scripts/package-macos.sh"
bridge_source="$repo_root/crates/spectrum-terminal/native/ghostty-bridge/Sources/SpectrumGhosttyBridge/SpectrumGhosttyBridge.swift"
surface_source="$repo_root/crates/spectrum-terminal/native/ghostty-bridge/Sources/SpectrumGhosttyBridge/SpectrumGhosttySurfaceView.swift"
focus_handoff_source="$repo_root/crates/spectrum-terminal/native/ghostty-bridge/Sources/SpectrumGhosttyBridge/SpectrumGhosttyFocusHandoff.swift"
focus_handoff_tests="$repo_root/crates/spectrum-terminal/native/ghostty-bridge/Tests/SpectrumGhosttyBridgeTests/SpectrumGhosttyFocusHandoffTests.swift"
fixture="$(mktemp -d "$repo_root/target/ghostty-source-tests.XXXXXX")"

[[ -x "$package_script" ]] || {
  echo "Prism packaging entrypoint is not executable: $package_script" >&2
  exit 1
}
[[ -x "$lumen_package_script" ]] || {
  echo "Lumen packaging entrypoint is not executable: $lumen_package_script" >&2
  exit 1
}
stale_proof_app_path="apps/prism/native/ghostty"'-proof'
stale_proof_lock_path="packaging/prism/macos/ghostty"'-proof'
! grep -R -n -F -e "$stale_proof_app_path" -e "$stale_proof_lock_path" \
  "$repo_root/.gitignore" "$repo_root/scripts" \
  "$repo_root/crates/spectrum-terminal" "$repo_root/THIRD_PARTY.md"

cleanup() {
  if [[ -n "${private_package_root:-}" && -d "$private_package_root" \
    && ! -L "$private_package_root" ]]; then
    case "$private_package_root" in
      "$repo_root"/target/spectrum-ghostty-package.*) rm -rf -- "$private_package_root" ;;
    esac
  fi
  if [[ -d "$fixture" && ! -L "$fixture" && "$(realpath "$fixture")" == "$fixture" ]]; then
    case "$fixture" in
      "$repo_root"/target/ghostty-source-tests.*) rm -rf -- "$fixture" ;;
    esac
  fi
}
trap cleanup EXIT

expect_failure() {
  local expected="$1"
  shift
  local output
  local exit_code
  set +e
  output="$("$@" 2>&1)"
  exit_code=$?
  set -e
  [[ $exit_code -ne 0 ]] || {
    echo "expected failure but command succeeded: $*" >&2
    exit 1
  }
  [[ "$output" == *"$expected"* ]] || {
    echo "failure did not contain '$expected': $output" >&2
    exit 1
  }
}

expect_exit_code() {
  local expected="$1"
  shift
  local exit_code
  set +e
  "$@" >/dev/null 2>&1
  exit_code=$?
  set -e
  [[ $exit_code -eq $expected ]] || {
    echo "expected exit $expected, got $exit_code: $*" >&2
    exit 1
  }
}

cp -- "$repo_root/packaging/spectrum-terminal/macos/ghostty-proof.lock" "$fixture/lock"
python3 "$metadata_validator" lock "$fixture/lock"
cp -- "$fixture/lock" "$fixture/duplicate-lock"
printf 'LOCK_FORMAT=2\n' >>"$fixture/duplicate-lock"
expect_failure "duplicate key: LOCK_FORMAT" \
  python3 "$metadata_validator" lock "$fixture/duplicate-lock"
cp -- "$fixture/lock" "$fixture/unknown-lock"
printf 'UNREVIEWED_KEY=value\n' >>"$fixture/unknown-lock"
expect_failure "unknown key: UNREVIEWED_KEY" \
  python3 "$metadata_validator" lock "$fixture/unknown-lock"
printf 'LOCK_FORMAT=2=extra\n' >"$fixture/malformed-lock"
expect_failure "not one KEY=VALUE assignment" \
  python3 "$metadata_validator" lock "$fixture/malformed-lock"

path_fixture="$fixture/path-binary"
printf 'first=%s second=%s\n' "$repo_root" "$fixture" >"$path_fixture"
chmod 0755 "$path_fixture"
python3 "$path_scrubber" "$path_fixture" "$repo_root" "$fixture"
! grep -aF -- "$repo_root" "$path_fixture" >/dev/null
! grep -aF -- "$fixture" "$path_fixture" >/dev/null
[[ "$(stat -f %Lp "$path_fixture")" == "755" ]]
expect_failure "absolute path" python3 "$path_scrubber" "$path_fixture" relative

zero_sha="0000000000000000000000000000000000000000000000000000000000000000"
cat >"$fixture/attestation" <<EOF
ATTESTATION_FORMAT=1
LOCK_FORMAT=2
PROOF_LOCK_SHA256=$zero_sha
PROOF_SCRIPT_SHA256=$zero_sha
TREE_HASHER_SHA256=$zero_sha
METADATA_VALIDATOR_SHA256=$zero_sha
XCFRAMEWORK_VALIDATOR_SHA256=$zero_sha
BOUNDED_RUNNER_SHA256=$zero_sha
SDK_TREE_HASHER_SHA256=$zero_sha
SDK_VALIDATOR_SHA256=$zero_sha
XCRUN_SHIM_SHA256=$zero_sha
GHOSTTY_VERSION=1.3.1
GHOSTTY_SOURCE_SHA256=$zero_sha
XCODE_VERSION=26.5
XCODE_BUILD=17F42
LLVM_LIBTOOL_SHA256=$zero_sha
CLT_MACOS_SDK_TREE_SHA256=$zero_sha
ZIG_VERSION=0.15.2
GHOSTTY_MACOS_TARGET=macos-arm64_x86_64
MINIMUM_MACOS_VERSION=13.0
SPECTRUM_GHOSTTY_BRIDGE_ABI_VERSION=1
GHOSTTY_MACOS_LIBRARY_SHA256=$zero_sha
GHOSTTY_MACOS_HEADER_SHA256=$zero_sha
GHOSTTY_XCFRAMEWORK_INFO_SHA256=$zero_sha
GHOSTTY_XCFRAMEWORK_TREE_SHA256=$zero_sha
GHOSTTY_RESOURCE_SENTINEL_SHA256=$zero_sha
GHOSTTY_RESOURCES_TREE_SHA256=$zero_sha
GHOSTTY_LICENSE_SHA256=$zero_sha
EOF
python3 "$metadata_validator" attestation "$fixture/attestation" \
  GHOSTTY_VERSION=1.3.1
expect_failure "value mismatch: GHOSTTY_VERSION" \
  python3 "$metadata_validator" attestation "$fixture/attestation" \
  GHOSTTY_VERSION=1.3.2
cp -- "$fixture/attestation" "$fixture/duplicate-attestation"
printf 'LOCK_FORMAT=2\n' >>"$fixture/duplicate-attestation"
expect_failure "duplicate key: LOCK_FORMAT" \
  python3 "$metadata_validator" attestation "$fixture/duplicate-attestation"
sed 's/^GHOSTTY_LICENSE_SHA256=.*/GHOSTTY_LICENSE_SHA256=bad/' \
  "$fixture/attestation" >"$fixture/invalid-sha-attestation"
expect_failure "invalid SHA-256: GHOSTTY_LICENSE_SHA256" \
  python3 "$metadata_validator" attestation "$fixture/invalid-sha-attestation"

mkdir -p "$fixture/trusted/Toolchains/XcodeDefault.xctoolchain/usr/bin" "$fixture/tool-shim"
trusted_tool="$fixture/trusted/Toolchains/XcodeDefault.xctoolchain/usr/bin/libtool"
printf 'pinned tool\n' >"$trusted_tool"
chmod 0755 "$trusted_tool"
trusted_tool_sha="$(shasum -a 256 "$trusted_tool" | awk '{print $1}')"
ln -s -- "$trusted_tool" "$fixture/tool-shim/libtool"
python3 "$metadata_validator" tool "$fixture/trusted" \
  Toolchains/XcodeDefault.xctoolchain/usr/bin/libtool \
  "$trusted_tool_sha" "$fixture/tool-shim/libtool"
expect_failure "relative path is unsafe" python3 "$metadata_validator" tool \
  "$fixture/trusted" ../libtool "$trusted_tool_sha" "$fixture/tool-shim/libtool"
expect_failure "checksum mismatch" python3 "$metadata_validator" tool \
  "$fixture/trusted" Toolchains/XcodeDefault.xctoolchain/usr/bin/libtool \
  "$zero_sha" "$fixture/tool-shim/libtool"
rm -- "$fixture/tool-shim/libtool"
ln -s -- /usr/bin/true "$fixture/tool-shim/libtool"
expect_failure "shim target mismatch" python3 "$metadata_validator" tool \
  "$fixture/trusted" Toolchains/XcodeDefault.xctoolchain/usr/bin/libtool \
  "$trusted_tool_sha" "$fixture/tool-shim/libtool"
rm -- "$fixture/tool-shim/libtool"
ln -s -- "$trusted_tool" "$fixture/tool-shim/libtool"
printf 'mutation\n' >>"$trusted_tool"
expect_failure "checksum mismatch" python3 "$metadata_validator" tool \
  "$fixture/trusted" Toolchains/XcodeDefault.xctoolchain/usr/bin/libtool \
  "$trusted_tool_sha" "$fixture/tool-shim/libtool"

runner_marker="$fixture/.live-process-group"
python3 "$bounded_runner" 2 "$runner_marker" -- /usr/bin/true
printf 'FORMAT=1\nPGID=1\n' >"$runner_marker"
expect_failure "live-process marker already exists" \
  python3 "$bounded_runner" 2 "$runner_marker" -- /usr/bin/true
rm -- "$runner_marker"
expect_failure "timeout must be a positive number" \
  python3 "$bounded_runner" 0 "$runner_marker" -- /usr/bin/true
expect_failure "timed out after 0.1 seconds" \
  python3 "$bounded_runner" 0.1 "$runner_marker" -- /bin/sh -c 'sleep 5'
[[ ! -e "$runner_marker" && ! -L "$runner_marker" ]]

cat >"$fixture/exited-leader-descendant.py" <<'PY'
import os
import subprocess
import sys
import time

child = """
import os
import signal
import sys
import time
signal.signal(signal.SIGTERM, signal.SIG_IGN)
with open(sys.argv[1], 'w', encoding='utf-8') as output:
    output.write(f'{os.getpid()} {os.getpgrp()}\\n')
time.sleep(60)
"""
subprocess.Popen([sys.executable, "-c", child, sys.argv[1]])
while not os.path.exists(sys.argv[1]):
    time.sleep(0.005)
PY
expect_failure "timed out after 2 seconds" \
  python3 "$bounded_runner" 2 "$runner_marker" -- \
  python3 "$fixture/exited-leader-descendant.py" "$fixture/exited-leader-child"
read -r exited_child_pid exited_child_pgid <"$fixture/exited-leader-child"
if kill -0 -- "-$exited_child_pgid" 2>/dev/null; then
  echo "bounded runner left exited leader's process group $exited_child_pgid live" >&2
  exit 1
fi
[[ ! -e "$runner_marker" && ! -L "$runner_marker" ]]

cat >"$fixture/term-resistant-descendant.py" <<'PY'
import signal
import subprocess
import sys
import time
import os

child = """
import os
import signal
import sys
import time
signal.signal(signal.SIGTERM, signal.SIG_IGN)
with open(sys.argv[1], 'w', encoding='utf-8') as output:
    output.write(f'{os.getpid()} {os.getpgrp()}\\n')
time.sleep(60)
"""
subprocess.Popen([sys.executable, "-c", child, sys.argv[1]])
while not os.path.exists(sys.argv[1]):
    time.sleep(0.005)
time.sleep(60)
PY
expect_failure "timed out after 0.1 seconds" \
  python3 "$bounded_runner" 0.1 "$runner_marker" -- \
  python3 "$fixture/term-resistant-descendant.py" "$fixture/descendant"
read -r descendant_pid descendant_pgid <"$fixture/descendant"
if kill -0 -- "-$descendant_pgid" 2>/dev/null; then
  echo "bounded runner left descendant process group $descendant_pgid live" >&2
  exit 1
fi
[[ ! -e "$runner_marker" && ! -L "$runner_marker" ]]

# Exercise the package cleanup function itself: any marker, including one for
# a currently live process group, must retain the private root for inspection.
cleanup_function="$fixture/package-cleanup.sh"
sed -n '/^cleanup_private_root() {/,/^}/p' "$package_script" >"$cleanup_function"
private_package_root="$(mktemp -d "$repo_root/target/spectrum-ghostty-package.XXXXXX")"
mkdir -- "$private_package_root/proof"
python3 - "$fixture/persistent-pgid" <<'PY' &
import os
import sys
import time

os.setsid()
with open(sys.argv[1], "w", encoding="utf-8") as output:
    output.write(f"{os.getpgrp()}\n")
    output.flush()
time.sleep(60)
PY
persistent_pid=$!
for _ in {1..100}; do
  [[ -s "$fixture/persistent-pgid" ]] && break
  sleep 0.01
done
persistent_pgid="$(<"$fixture/persistent-pgid")"
PYTHONDONTWRITEBYTECODE=1 python3 - \
  "$bounded_runner" "$private_package_root/proof/.live-process-group" \
  "$persistent_pgid" <<'PY'
import importlib.util
import pathlib
import sys

spec = importlib.util.spec_from_file_location("spectrum_bounded_runner", sys.argv[1])
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
module.write_live_marker(pathlib.Path(sys.argv[2]), int(sys.argv[3]))
PY
[[ "$(stat -f %Lp "$private_package_root/proof/.live-process-group")" == "600" ]]
[[ "$(<"$private_package_root/proof/.live-process-group")" == \
  $'FORMAT=1\nPGID='"$persistent_pgid" ]]
private_root="$private_package_root"
# shellcheck source=/dev/null
source "$cleanup_function"
cleanup_private_root
[[ -d "$private_package_root" ]]
kill -KILL -- "-$persistent_pgid"
wait "$persistent_pid" 2>/dev/null || true
rm -rf -- "$private_package_root"
unset private_root private_package_root persistent_pid persistent_pgid

sdk_fixture="$fixture/MacOSX15.2.sdk"
mkdir -p "$sdk_fixture/usr/lib" "$sdk_fixture/usr/include" "$sdk_fixture/links"
python3 - "$sdk_fixture/SDKSettings.plist" <<'PY'
import plistlib
import sys

with open(sys.argv[1], "wb") as destination:
    plistlib.dump({"CanonicalName": "macosx15.2", "Version": "15.2"}, destination)
PY
printf '%s\n' '--- !tapi-tbd' 'targets: [ x86_64-macos, arm64-macos ]' \
  "install-name: '/usr/lib/libSystem.B.dylib'" \
  >"$sdk_fixture/usr/lib/libSystem.tbd"
printf 'libc++\n' >"$sdk_fixture/usr/lib/libc++.tbd"
printf 'header\n' >"$sdk_fixture/usr/include/example.h"
printf 'target-a\n' >"$sdk_fixture/links/a"
printf 'target-b\n' >"$sdk_fixture/links/b"
ln -s a "$sdk_fixture/links/current"
sdk_tree_sha="$(python3 "$sdk_tree_hasher" "$sdk_fixture")"
sdk_settings_sha="$(shasum -a 256 "$sdk_fixture/SDKSettings.plist" | awk '{print $1}')"
sdk_libsystem_sha="$(shasum -a 256 "$sdk_fixture/usr/lib/libSystem.tbd" | awk '{print $1}')"
sdk_libcxx_sha="$(shasum -a 256 "$sdk_fixture/usr/lib/libc++.tbd" | awk '{print $1}')"
sdk_uid="$(stat -f %u "$sdk_fixture")"
sdk_gid="$(stat -f %g "$sdk_fixture")"
sdk_verify=(python3 "$sdk_validator" "$sdk_fixture" macosx15.2 15.2 \
  "$sdk_tree_sha" "$sdk_settings_sha" "$sdk_libsystem_sha" "$sdk_libcxx_sha" \
  "$sdk_tree_hasher" "$sdk_uid" "$sdk_gid" spectrum-sdk-v1)
"${sdk_verify[@]}"
expect_failure "canonical name mismatch" python3 "$sdk_validator" \
  "$sdk_fixture" macosx15.1 15.2 "$sdk_tree_sha" "$sdk_settings_sha" \
  "$sdk_libsystem_sha" "$sdk_libcxx_sha" "$sdk_tree_hasher" \
  "$sdk_uid" "$sdk_gid" spectrum-sdk-v1
expect_failure "version mismatch" python3 "$sdk_validator" \
  "$sdk_fixture" macosx15.2 15.1 "$sdk_tree_sha" "$sdk_settings_sha" \
  "$sdk_libsystem_sha" "$sdk_libcxx_sha" "$sdk_tree_hasher" \
  "$sdk_uid" "$sdk_gid" spectrum-sdk-v1
expect_failure "key file checksum mismatch" python3 "$sdk_validator" \
  "$sdk_fixture" macosx15.2 15.2 "$sdk_tree_sha" "$sdk_settings_sha" \
  "$zero_sha" "$sdk_libcxx_sha" "$sdk_tree_hasher" \
  "$sdk_uid" "$sdk_gid" spectrum-sdk-v1
ln -s "$sdk_fixture" "$fixture/sdk-root-link"
expect_failure "absolute canonical real directory" python3 "$sdk_validator" \
  "$fixture/sdk-root-link" macosx15.2 15.2 "$sdk_tree_sha" "$sdk_settings_sha" \
  "$sdk_libsystem_sha" "$sdk_libcxx_sha" "$sdk_tree_hasher" \
  "$sdk_uid" "$sdk_gid" spectrum-sdk-v1
rm -- "$fixture/sdk-root-link"
expect_failure "SDK node ownership mismatch" python3 "$sdk_validator" \
  "$sdk_fixture" macosx15.2 15.2 "$sdk_tree_sha" "$sdk_settings_sha" \
  "$sdk_libsystem_sha" "$sdk_libcxx_sha" "$sdk_tree_hasher" \
  "$((sdk_uid + 1))" "$sdk_gid" spectrum-sdk-v1
chmod 0664 "$sdk_fixture/usr/include/example.h"
expect_failure "group- or other-writable" "${sdk_verify[@]}"
chmod 0644 "$sdk_fixture/usr/include/example.h"
printf 'mutation\n' >>"$sdk_fixture/usr/include/example.h"
expect_failure "complete SDK tree checksum mismatch" "${sdk_verify[@]}"
printf 'header\n' >"$sdk_fixture/usr/include/example.h"
chmod 0600 "$sdk_fixture/usr/include/example.h"
expect_failure "complete SDK tree checksum mismatch" "${sdk_verify[@]}"
chmod 0644 "$sdk_fixture/usr/include/example.h"
rm -- "$sdk_fixture/links/current"
ln -s b "$sdk_fixture/links/current"
expect_failure "complete SDK tree checksum mismatch" "${sdk_verify[@]}"
rm -- "$sdk_fixture/links/current"
ln -s /tmp "$sdk_fixture/links/current"
expect_failure "unsafe target" python3 "$sdk_tree_hasher" "$sdk_fixture"
rm -- "$sdk_fixture/links/current"
ln -s ../../../escape "$sdk_fixture/links/current"
expect_failure "escapes the root" python3 "$sdk_tree_hasher" "$sdk_fixture"
rm -- "$sdk_fixture/links/current"
ln -s a "$sdk_fixture/links/current"

xcrun_result="$(SPECTRUM_GHOSTTY_MACOS_SDK_ROOT="$sdk_fixture" \
  bash "$xcrun_shim" --sdk macosx --show-sdk-path)"
[[ "$xcrun_result" == "$sdk_fixture" ]]
expect_failure "" env SPECTRUM_GHOSTTY_MACOS_SDK_ROOT="$sdk_fixture" \
  bash "$xcrun_shim" --sdk macosx --show-sdk-path --extra
expected_clang="$(DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer \
  /usr/bin/xcrun --find clang)"
actual_clang="$(DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer \
  SPECTRUM_GHOSTTY_MACOS_SDK_ROOT="$sdk_fixture" bash "$xcrun_shim" --find clang)"
[[ "$actual_clang" == "$expected_clang" ]]

python3 - "$fixture/Info.plist" <<'PY'
import plistlib
import sys

payload = {
    "AvailableLibraries": [{
        "LibraryIdentifier": "macos-arm64_x86_64",
        "LibraryPath": "libghostty.a",
        "BinaryPath": "libghostty.a",
        "HeadersPath": "Headers",
        "SupportedArchitectures": ["arm64", "x86_64"],
        "SupportedPlatform": "macos",
    }],
    "XCFrameworkFormatVersion": "1.0",
}
with open(sys.argv[1], "wb") as destination:
    plistlib.dump(payload, destination)
PY
python3 "$xcframework_validator" "$fixture/Info.plist" macos-arm64_x86_64
for mutation in platform library architectures; do
  python3 - "$fixture/Info.plist" "$fixture/Info-$mutation.plist" "$mutation" <<'PY'
import plistlib
import sys

with open(sys.argv[1], "rb") as source:
    payload = plistlib.load(source)
entry = payload["AvailableLibraries"][0]
if sys.argv[3] == "platform":
    entry["SupportedPlatform"] = "ios"
elif sys.argv[3] == "library":
    entry["LibraryPath"] = "../escape.a"
else:
    entry["SupportedArchitectures"] = ["arm64"]
with open(sys.argv[2], "wb") as destination:
    plistlib.dump(payload, destination)
PY
done
expect_failure "not a native macOS slice" \
  python3 "$xcframework_validator" "$fixture/Info-platform.plist" macos-arm64_x86_64
expect_failure "library path is unexpected" \
  python3 "$xcframework_validator" "$fixture/Info-library.plist" macos-arm64_x86_64
expect_failure "architectures are not arm64 and x86_64" \
  python3 "$xcframework_validator" "$fixture/Info-architectures.plist" macos-arm64_x86_64

mkdir -p "$fixture/tree/empty"
printf 'alpha\n' >"$fixture/tree/artifact"
chmod 0644 "$fixture/tree/artifact"
tree_hash="$(python3 "$tree_hasher" "$fixture/tree")"
printf 'beta\n' >>"$fixture/tree/artifact"
mutated_hash="$(python3 "$tree_hasher" "$fixture/tree")"
[[ "$tree_hash" != "$mutated_hash" ]]
printf 'alpha\n' >"$fixture/tree/artifact"
chmod 0600 "$fixture/tree/artifact"
permission_hash="$(python3 "$tree_hasher" "$fixture/tree")"
[[ "$tree_hash" != "$permission_hash" ]]
chmod 0644 "$fixture/tree/artifact"
mkdir "$fixture/tree/another-empty"
topology_hash="$(python3 "$tree_hasher" "$fixture/tree")"
[[ "$tree_hash" != "$topology_hash" ]]
rmdir "$fixture/tree/another-empty"
mkfifo "$fixture/tree/fifo"
expect_failure "symlink or special node: fifo" python3 "$tree_hasher" "$fixture/tree"
rm -- "$fixture/tree/fifo"
ln -s artifact "$fixture/tree/link"
expect_failure "symlink or special node: link" python3 "$tree_hasher" "$fixture/tree"
rm -- "$fixture/tree/link"
ln -s tree "$fixture/tree-root-link"
expect_failure "root is not a real directory" python3 "$tree_hasher" "$fixture/tree-root-link"
rm -- "$fixture/tree-root-link"
printf 'newline\n' >"$fixture/tree/"$'bad\nname'
expect_failure "path contains a newline" python3 "$tree_hasher" "$fixture/tree"
rm -- "$fixture/tree/"$'bad\nname'
cp -R "$fixture/tree" "$fixture/overlay"
python3 "$tree_hasher" --verify-overlay "$fixture/tree" "$fixture/overlay"
printf 'tamper\n' >>"$fixture/overlay/artifact"
expect_failure "overlay artifact content differs: artifact" \
  python3 "$tree_hasher" --verify-overlay "$fixture/tree" "$fixture/overlay"
cp -- "$fixture/tree/artifact" "$fixture/overlay/artifact"
chmod 0600 "$fixture/overlay/artifact"
expect_failure "overlay artifact mode differs: artifact" \
  python3 "$tree_hasher" --verify-overlay "$fixture/tree" "$fixture/overlay"

mkdir "$fixture/shims"
cat >"$fixture/shims/tool" <<'PY'
#!/usr/bin/env python3
import os
import sys

tool = os.path.basename(sys.argv[0])
mode = os.environ.get("SPECTRUM_GHOSTTY_TEST_MODE", "ok")
symbols = [
    "_spectrum_ghostty_bridge_abi_version",
    "_spectrum_ghostty_global_init",
    "_spectrum_ghostty_runtime_create",
    "_spectrum_ghostty_runtime_tick",
    "_spectrum_ghostty_runtime_set_focus",
    "_spectrum_ghostty_runtime_destroy",
    "_spectrum_ghostty_surface_create",
    "_spectrum_ghostty_surface_set_state",
    "_spectrum_ghostty_surface_edit",
    "_spectrum_ghostty_surface_request_close",
    "_spectrum_ghostty_surface_destroy",
]
if tool == "otool":
    if sys.argv[1] == "-D":
        print(sys.argv[2])
        print("@rpath/wrong.dylib" if mode == "install-name" else "@rpath/libSpectrumGhosttyBridge.dylib")
    elif sys.argv[1] == "-L":
        print(sys.argv[2] + ":")
        dependency = "/tmp/unreviewed.dylib" if mode == "dependency" else "/usr/lib/libSystem.B.dylib"
        print(f"\t{dependency} (compatibility version 1.0.0, current version 1.0.0)")
    elif sys.argv[1] == "-l":
        print("cmd LC_BUILD_VERSION")
        print("minos " + ("12.0" if mode == "minos" else "13.0"))
elif tool == "lipo":
    print("x86_64" if mode == "architecture" else "arm64")
elif tool == "nm":
    if mode == "nm-error":
        print("synthetic archive parse failure", file=sys.stderr)
        raise SystemExit(47)
    for symbol in symbols:
        if mode != "abi" or symbol != "_spectrum_ghostty_surface_edit":
            print("0000000000000000 T " + symbol)
elif tool == "strings":
    if mode == "leakage":
        print("/forbidden/build/path")
elif tool == "codesign":
    raise SystemExit(1 if mode == "signature" else 0)
PY
chmod 0755 "$fixture/shims/tool"
for tool in otool lipo nm strings codesign; do
  ln -s tool "$fixture/shims/$tool"
done
printf 'fake Mach-O\n' >"$fixture/libSpectrumGhosttyBridge.dylib"
bridge_command=(bash "$bridge_verifier" "$fixture/libSpectrumGhosttyBridge.dylib" 13.0 arm64 /forbidden/build/path)
PATH="$fixture/shims:/usr/bin:/bin" SPECTRUM_GHOSTTY_TEST_MODE=ok "${bridge_command[@]}"
symbol_check=(python3 "$metadata_validator" symbols "$fixture/shims/nm" \
  "$fixture/libSpectrumGhosttyBridge.dylib" arm64 \
  _spectrum_ghostty_bridge_abi_version _spectrum_ghostty_surface_edit)
symbol_output="$(SPECTRUM_GHOSTTY_TEST_MODE=ok "${symbol_check[@]}")"
[[ "$symbol_output" == *'"status": "ok"'* ]]
[[ "$symbol_output" == *'"architecture": "arm64"'* ]]
[[ "$symbol_output" == *'"artifact_sha256":'* ]]
[[ "$symbol_output" == *'"tool_sha256":'* ]]
expect_failure '"status": "missing_symbols"' env SPECTRUM_GHOSTTY_TEST_MODE=abi \
  "${symbol_check[@]}"
expect_failure '"missing_symbols": ["_spectrum_ghostty_surface_edit"]' env \
  SPECTRUM_GHOSTTY_TEST_MODE=abi "${symbol_check[@]}"
expect_exit_code 3 env SPECTRUM_GHOSTTY_TEST_MODE=abi "${symbol_check[@]}"
expect_failure '"status": "tool_error"' env SPECTRUM_GHOSTTY_TEST_MODE=nm-error \
  "${symbol_check[@]}"
expect_failure '"tool_exit_code": 47' env SPECTRUM_GHOSTTY_TEST_MODE=nm-error \
  "${symbol_check[@]}"
expect_failure 'synthetic archive parse failure' env SPECTRUM_GHOSTTY_TEST_MODE=nm-error \
  "${symbol_check[@]}"
expect_exit_code 2 env SPECTRUM_GHOSTTY_TEST_MODE=nm-error "${symbol_check[@]}"
mkdir "$fixture/real-bridge-parent"
cp -- "$fixture/libSpectrumGhosttyBridge.dylib" "$fixture/real-bridge-parent/bridge.dylib"
ln -s real-bridge-parent "$fixture/symlinked-bridge-parent"
expect_failure "symlink or non-canonical component" env \
  PATH="$fixture/shims:/usr/bin:/bin" SPECTRUM_GHOSTTY_TEST_MODE=ok \
  bash "$bridge_verifier" "$fixture/symlinked-bridge-parent/bridge.dylib" \
  13.0 arm64 /forbidden/build/path
for test_case in \
  'install-name:unexpected install name' \
  'dependency:non-allowlisted dependency' \
  'minos:deployment target' \
  'architecture:does not contain architecture' \
  'abi:missing_symbols' \
  'nm-error:tool_error' \
  'leakage:retained forbidden build path'; do
  mode="${test_case%%:*}"
  expected="${test_case#*:}"
  expect_failure "$expected" env PATH="$fixture/shims:/usr/bin:/bin" \
    SPECTRUM_GHOSTTY_TEST_MODE="$mode" "${bridge_command[@]}"
done
expect_failure "" env PATH="$fixture/shims:/usr/bin:/bin" \
  SPECTRUM_GHOSTTY_TEST_MODE=signature "${bridge_command[@]}"

# A caller cannot authorize a forged artifact plus matching attestation by
# passing an external proof root to either production entry point.
expect_failure "usage:" bash "$repo_root/scripts/package-prism-macos.sh" \
  --with-ghostty "$fixture/forged-proof"
expect_failure "usage:" bash "$repo_root/scripts/package-macos.sh" \
  --with-ghostty "$fixture/forged-proof"
private_package_root="$(mktemp -d "$repo_root/target/spectrum-ghostty-package.XXXXXX")"
chmod 0700 "$private_package_root"
mkdir -p "$fixture/forged-proof"
expect_failure "rejects externally prepared proof roots" env \
  DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer \
  MACOSX_DEPLOYMENT_TARGET=13.0 \
  SPECTRUM_GHOSTTY_PRIVATE_ROOT="$private_package_root" \
  bash "$repo_root/scripts/build-spectrum-ghostty-bridge-macos.sh" \
  "$fixture/forged-proof" "$private_package_root/bridge"
rmdir "$fixture/forged-proof" "$private_package_root"

python3 - "$repo_root/scripts/build-spectrum-ghostty-macos.sh" \
  "$repo_root/scripts/build-spectrum-ghostty-bridge-macos.sh" \
  "$repo_root/scripts/package-prism-macos.sh" \
  "$repo_root/scripts/package-macos.sh" \
  "$bridge_source" "$surface_source" \
  "$focus_handoff_source" "$focus_handoff_tests" <<'PY'
import sys
from pathlib import Path

proof = Path(sys.argv[1]).read_text()
consumer = Path(sys.argv[2]).read_text()
packages = [Path(sys.argv[3]).read_text(), Path(sys.argv[4]).read_text()]
bridge_source = Path(sys.argv[5]).read_text()
surface_source = Path(sys.argv[6]).read_text()
focus_handoff_source = Path(sys.argv[7]).read_text()
focus_handoff_tests = Path(sys.argv[8]).read_text()
zig = proof.index('zig_source="$toolchains_dir/zig-$zig_arch-macos-$zig_version"')
remove = proof.index('safe_remove_tree "$zig_source"', zig)
extract = proof.index('extract_once "$zig_archive"', remove)
assert zig < remove < extract
assert 'mktemp "$destination.part.XXXXXX"' in proof
assert 'canonical_storage_root' in proof and 'realpath "$target"' in proof
assert proof.count("verify_pinned_archive_tool") >= 5
assert 'run_bounded 120 "${zig_command[@]}" version' in proof
assert 'run_bounded 3600 env' in proof
assert 'PATH="$libtool_shim_dir:$PATH"' in proof
assert 'HOMEBREW_LLVM_LIBTOOL_RELATIVE' in proof
assert 'LLVM_LIBTOOL_SHA256' in proof
assert '"${zig_command[@]}" build -j2' in proof
assert 'zig_url="$(lock_value ZIG_ARM64_URL)"' in proof
assert '/usr/bin/arch -x86_64' not in proof
assert proof.count("verify_pinned_sdk_and_shim") >= 5
assert 'python3 "$metadata_validator" symbols' in proof
assert '2>/dev/null' not in proof[proof.index('for architecture in arm64 x86_64; do'):]
assert 'python3 "$metadata_validator" symbols' in consumer
assert 'python3 "$path_scrubber" "$bridge"' in consumer
assert consumer.count("verify_snapshot_hashes") >= 2
assert consumer.count("verify_sealed_snapshot_unchanged") >= 4
assert 'ln -s -- "$xcframework"' not in consumer
assert 'scratch="$stage/swift-build"' in consumer
assert '"$repo_root=/spectrum"' in consumer
assert '"-ffile-prefix-map=$repo_root=/spectrum"' in consumer
for package in packages:
    assert 'bash "$proof_builder" --storage-root "$proof_root"' in package
    assert 'chain_path_scrubber_sha' in package
    assert 'mktemp -d "$repo_root/target/spectrum-ghostty-package.XXXXXX"' in package
    assert '"$repo_root"/target/spectrum-ghostty-package.*)' in package
    assert 'cargo_features' not in package
    assert 'cargo_features[@]' not in package
    assert 'chmod -R u+w "$bundle"' in package
    assert 'refusing to replace unsafe bundle path' in package
    assert 'local exit_code=$?' in package
    assert 'return "$exit_code"' in package
assert 'cargo build --release --locked -p prism --bins --features ghostty-terminal' in packages[0]
assert 'cargo build --release --locked -p prism --bins\n' in packages[0]
assert 'cargo build --release --locked -p lumen-photo --bins --features ghostty-terminal' in packages[1]
assert 'cargo build --release --locked -p lumen-photo --bins\n' in packages[1]
assert '"$proof_root" == "$private_root/proof"' in consumer
assert 'bridge packaging rejects externally prepared proof roots' in consumer
assert all('verified-proof-root' not in package for package in packages)
assert 'public typealias SpectrumGhosttyEventCallback' in bridge_source
assert '@convention(c)' in bridge_source
assert surface_source.count('guard ownsPresentedFocus else { return }') >= 2
assert 'ghostty_surface_set_focus(surface, ownsPresentedFocus)' in surface_source
assert surface_source.count('SpectrumGhosttyFocusHandoff.applyVisibility') >= 3
destroy = surface_source.index('func destroySurface()')
free = surface_source.index('ghostty_surface_free(surface)', destroy)
restore = surface_source.index('SpectrumGhosttyFocusHandoff.applyVisibility(false', destroy)
assert destroy < restore < free
restore = focus_handoff_source.index('restoreHostResponderIfOwned')
hide = focus_handoff_source.index('surfaceView.isHidden = !visible')
assert restore < hide
assert 'window.firstResponder === surfaceView' in focus_handoff_source
assert 'for _ in 0..<4' in focus_handoff_tests
assert 'window.firstResponder === host' in focus_handoff_tests
assert 'window.firstResponder === editor' in focus_handoff_tests
test_copy = consumer.index('cp -R -- "$bridge_source/Tests"')
swift_test = consumer.index('xcrun swift test', test_copy)
swift_build = consumer.index('xcrun swift build', swift_test)
assert test_copy < swift_test < swift_build
assert 'apps/prism/' not in packages[1]
assert 'packaging/prism/' not in packages[1]
assert 'scripts/build-prism' not in packages[1]
PY

echo "Spectrum Ghostty packaging source checks passed"
