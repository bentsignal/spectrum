#!/usr/bin/env python3
"""Strict schema validation for Spectrum's Ghostty lock and attestation files."""

from __future__ import annotations

import hashlib
import json
import os
import re
import subprocess
import sys
import tempfile
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
    "SPECTRUM_GHOSTTY_BRIDGE_ABI_VERSION",
    "GHOSTTY_XCFRAMEWORK_PATH",
    "GHOSTTY_RESOURCES_PATH",
    "GHOSTTY_RESOURCE_SENTINEL",
    "GHOSTTY_PROOF_ATTESTATION",
    "XCODE_VERSION",
    "XCODE_BUILD",
    "HOMEBREW_LLVM_FORMULA",
    "HOMEBREW_LLVM_VERSION",
    "HOMEBREW_LLVM_ARCH",
    "HOMEBREW_LLVM_PREFIX",
    "HOMEBREW_LLVM_LIBTOOL_RELATIVE",
    "LLVM_LIBTOOL_SHA256",
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
    "LLVM_LIBTOOL_SHA256",
    "CLT_MACOS_SDK_TREE_SHA256",
    "ZIG_VERSION",
    "GHOSTTY_MACOS_TARGET",
    "MINIMUM_MACOS_VERSION",
    "SPECTRUM_GHOSTTY_BRIDGE_ABI_VERSION",
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
SYMBOL_DIAGNOSTIC_LIMIT = 4096


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


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def diagnostic_excerpt(stream: object) -> str:
    stream.seek(0)
    value = stream.read(SYMBOL_DIAGNOSTIC_LIMIT + 1)
    truncated = len(value) > SYMBOL_DIAGNOSTIC_LIMIT
    value = value[:SYMBOL_DIAGNOSTIC_LIMIT]
    text = value.decode("utf-8", errors="replace")
    return text + ("\n[truncated]" if truncated else "")


def validate_symbols(
    tool_argument: str,
    artifact: Path,
    architecture: str,
    required_symbols: list[str],
) -> None:
    tool = Path(tool_argument)
    if not tool.is_absolute() or not tool.exists() or not os.access(tool, os.X_OK):
        fail("symbol tool must be an absolute executable path")
    try:
        resolved_tool = tool.resolve(strict=True)
    except OSError as error:
        fail(f"could not resolve symbol tool: {error}")
    if not resolved_tool.is_file():
        fail("resolved symbol tool is not a regular file")
    if not artifact.is_absolute() or not artifact.is_file() or artifact.is_symlink():
        fail("symbol artifact must be an absolute regular non-symlink file")
    try:
        resolved_artifact = artifact.resolve(strict=True)
    except OSError as error:
        fail(f"could not resolve symbol artifact: {error}")
    if resolved_artifact != artifact:
        fail("symbol artifact has a symlinked or non-canonical component")
    if architecture not in {"arm64", "x86_64"}:
        fail("symbol architecture must be arm64 or x86_64")
    if not required_symbols or any(
        not symbol.startswith("_") or any(character.isspace() for character in symbol)
        for symbol in required_symbols
    ):
        fail("required symbols must be non-empty underscore-prefixed names")

    record: dict[str, object] = {
        "architecture": architecture,
        "artifact": str(artifact),
        "artifact_sha256": sha256_file(artifact),
        "required_symbols": required_symbols,
        "tool": str(tool),
        "tool_resolved": str(resolved_tool),
        "tool_sha256": sha256_file(resolved_tool),
    }
    with tempfile.TemporaryFile() as stdout, tempfile.TemporaryFile() as stderr:
        try:
            completed = subprocess.run(
                [str(tool), "-arch", architecture, "-gU", str(artifact)],
                stdin=subprocess.DEVNULL,
                stdout=stdout,
                stderr=stderr,
                check=False,
            )
        except OSError as error:
            record.update({"status": "tool_error", "launch_error": str(error)})
            print(f"spectrum-ghostty-symbol-check {json.dumps(record, sort_keys=True)}", file=sys.stderr)
            raise SystemExit(2)

        record["tool_exit_code"] = completed.returncode
        stderr_excerpt = diagnostic_excerpt(stderr)
        if stderr_excerpt:
            record["tool_stderr"] = stderr_excerpt
        if completed.returncode != 0:
            stdout_excerpt = diagnostic_excerpt(stdout)
            if stdout_excerpt:
                record["tool_stdout"] = stdout_excerpt
            record["status"] = "tool_error"
            print(f"spectrum-ghostty-symbol-check {json.dumps(record, sort_keys=True)}", file=sys.stderr)
            raise SystemExit(2)

        stdout.seek(0)
        observed = set()
        for raw_line in stdout:
            fields = raw_line.decode("utf-8", errors="replace").split()
            if fields:
                observed.add(fields[-1])
        missing = [symbol for symbol in required_symbols if symbol not in observed]
        record["observed_symbol_count"] = len(observed)
        if missing:
            record.update({"status": "missing_symbols", "missing_symbols": missing})
            print(f"spectrum-ghostty-symbol-check {json.dumps(record, sort_keys=True)}", file=sys.stderr)
            raise SystemExit(3)
        record["status"] = "ok"
        print(f"spectrum-ghostty-symbol-check {json.dumps(record, sort_keys=True)}")


def main() -> None:
    if len(sys.argv) >= 6 and sys.argv[1] == "symbols":
        validate_symbols(sys.argv[2], Path(sys.argv[3]), sys.argv[4], sys.argv[5:])
        return
    if len(sys.argv) == 6 and sys.argv[1] == "tool":
        validate_tool(Path(sys.argv[2]), sys.argv[3], sys.argv[4], Path(sys.argv[5]))
        return
    if len(sys.argv) < 3 or sys.argv[1] not in {"lock", "attestation"}:
        fail(
            f"usage: {Path(sys.argv[0]).name} lock|attestation <path> [KEY=VALUE ...] "
            "or tool <trusted-root> <relative-tool> <sha256> <shim> "
            "or symbols <nm-tool> <artifact> <arm64|x86_64> <symbol...>"
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
