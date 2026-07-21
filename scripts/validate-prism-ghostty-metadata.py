#!/usr/bin/env python3
"""Strict schema validation for Prism's Ghostty lock and attestation files."""

from __future__ import annotations

import hashlib
import os
import re
import sys
from pathlib import Path


LOCK_KEYS = {
    "LOCK_FORMAT",
    "GHOSTTY_VERSION",
    "GHOSTTY_TAG",
    "GHOSTTY_TAG_OBJECT",
    "GHOSTTY_REVISION",
    "GHOSTTY_SOURCE_URL",
    "GHOSTTY_SOURCE_SHA256",
    "ZIG_VERSION",
    "ZIG_ARM64_URL",
    "ZIG_ARM64_SHA256",
    "ZIG_X86_64_URL",
    "ZIG_X86_64_SHA256",
    "CLT_MACOS_SDK_PATH",
    "CLT_MACOS_SDK_CANONICAL_NAME",
    "CLT_MACOS_SDK_VERSION",
    "CLT_MACOS_SDK_UID",
    "CLT_MACOS_SDK_GID",
    "CLT_MACOS_SDK_TREE_SHA256",
    "CLT_MACOS_SDK_SETTINGS_SHA256",
    "CLT_MACOS_SDK_LIBSYSTEM_SHA256",
    "CLT_MACOS_SDK_LIBCXX_SHA256",
    "MINIMUM_MACOS_VERSION",
    "GHOSTTY_MACOS_TARGET",
    "PRISM_GHOSTTY_BRIDGE_ABI_VERSION",
    "GHOSTTY_XCFRAMEWORK_PATH",
    "GHOSTTY_RESOURCES_PATH",
    "GHOSTTY_RESOURCE_SENTINEL",
    "GHOSTTY_PROOF_ATTESTATION",
    "XCODE_VERSION",
    "XCODE_BUILD",
    "XCODE_LIBTOOL_SHA256",
}

ATTESTATION_KEYS = {
    "ATTESTATION_FORMAT",
    "LOCK_FORMAT",
    "PROOF_LOCK_SHA256",
    "PROOF_SCRIPT_SHA256",
    "TREE_HASHER_SHA256",
    "METADATA_VALIDATOR_SHA256",
    "XCFRAMEWORK_VALIDATOR_SHA256",
    "BOUNDED_RUNNER_SHA256",
    "SDK_TREE_HASHER_SHA256",
    "SDK_VALIDATOR_SHA256",
    "XCRUN_SHIM_SHA256",
    "GHOSTTY_VERSION",
    "GHOSTTY_SOURCE_SHA256",
    "XCODE_VERSION",
    "XCODE_BUILD",
    "XCODE_LIBTOOL_SHA256",
    "CLT_MACOS_SDK_TREE_SHA256",
    "ZIG_VERSION",
    "GHOSTTY_MACOS_TARGET",
    "MINIMUM_MACOS_VERSION",
    "PRISM_GHOSTTY_BRIDGE_ABI_VERSION",
    "GHOSTTY_MACOS_LIBRARY_SHA256",
    "GHOSTTY_MACOS_HEADER_SHA256",
    "GHOSTTY_XCFRAMEWORK_INFO_SHA256",
    "GHOSTTY_XCFRAMEWORK_TREE_SHA256",
    "GHOSTTY_RESOURCE_SENTINEL_SHA256",
    "GHOSTTY_RESOURCES_TREE_SHA256",
    "GHOSTTY_LICENSE_SHA256",
}

KEY_PATTERN = re.compile(r"^[A-Z0-9_]+$")
SHA_PATTERN = re.compile(r"^[0-9a-f]{64}$")


def fail(message: str) -> "NoReturn":
    print(f"error: {message}", file=sys.stderr)
    raise SystemExit(1)


def validate(kind: str, path: Path) -> dict[str, str]:
    expected = LOCK_KEYS if kind == "lock" else ATTESTATION_KEYS
    try:
        raw = path.read_bytes()
    except OSError as error:
        fail(f"could not read {kind}: {error}")
    if not raw.endswith(b"\n") or b"\r" in raw or b"\0" in raw:
        fail(f"{kind} must be newline-terminated UTF-8 without CR or NUL bytes")
    try:
        lines = raw.decode("utf-8", errors="strict").splitlines()
    except UnicodeDecodeError:
        fail(f"{kind} is not valid UTF-8")

    values: dict[str, str] = {}
    for number, line in enumerate(lines, start=1):
        if kind == "lock" and (not line.strip() or line.lstrip().startswith("#")):
            continue
        if kind == "attestation" and not line:
            fail(f"{kind} line {number} is blank")
        if line.count("=") != 1:
            fail(f"{kind} line {number} is not one KEY=VALUE assignment")
        key, value = line.split("=", 1)
        if not KEY_PATTERN.fullmatch(key) or not value:
            fail(f"{kind} line {number} has an invalid key or empty value")
        if key not in expected:
            fail(f"{kind} contains unknown key: {key}")
        if key in values:
            fail(f"{kind} contains duplicate key: {key}")
        if kind == "attestation" and key.endswith("_SHA256") and not SHA_PATTERN.fullmatch(value):
            fail(f"attestation contains invalid SHA-256: {key}")
        values[key] = value

    missing = sorted(expected - values.keys())
    if missing:
        fail(f"{kind} is missing keys: {', '.join(missing)}")
    return values


def validate_tool(
    trusted_root: Path,
    relative_tool: str,
    expected_sha: str,
    shim: Path,
) -> None:
    if not trusted_root.is_absolute() or not trusted_root.is_dir() or trusted_root.is_symlink():
        fail("trusted tool root is not an absolute real directory")
    if trusted_root.resolve() != trusted_root:
        fail("trusted tool root has a symlinked or non-canonical component")
    relative = Path(relative_tool)
    if relative.is_absolute() or not relative.parts or ".." in relative.parts:
        fail("trusted tool relative path is unsafe")
    tool = trusted_root / relative
    if not tool.is_file() or tool.is_symlink() or tool.resolve() != tool:
        fail("trusted tool is missing, symlinked, or non-canonical")
    try:
        tool.relative_to(trusted_root)
    except ValueError:
        fail("trusted tool escapes its root")
    if not os.access(tool, os.X_OK):
        fail("trusted tool is not executable")
    if not SHA_PATTERN.fullmatch(expected_sha):
        fail("trusted tool expected SHA-256 is malformed")
    digest = hashlib.sha256()
    with tool.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    if digest.hexdigest() != expected_sha:
        fail("trusted tool checksum mismatch")
    if not shim.is_absolute() or not shim.is_symlink():
        fail("trusted tool shim is not an absolute symlink")
    if os.readlink(shim) != str(tool) or shim.resolve() != tool:
        fail("trusted tool shim target mismatch")


def main() -> None:
    if len(sys.argv) == 6 and sys.argv[1] == "tool":
        validate_tool(Path(sys.argv[2]), sys.argv[3], sys.argv[4], Path(sys.argv[5]))
        return
    if len(sys.argv) < 3 or sys.argv[1] not in {"lock", "attestation"}:
        fail(
            f"usage: {Path(sys.argv[0]).name} lock|attestation <path> [KEY=VALUE ...] "
            "or tool <trusted-root> <relative-tool> <sha256> <shim>"
        )
    values = validate(sys.argv[1], Path(sys.argv[2]))
    for expectation in sys.argv[3:]:
        if expectation.count("=") != 1:
            fail(f"invalid expected assignment: {expectation}")
        key, expected = expectation.split("=", 1)
        if key not in values:
            fail(f"cannot compare unknown expected key: {key}")
        if values[key] != expected:
            fail(f"{sys.argv[1]} value mismatch: {key}")


if __name__ == "__main__":
    main()
