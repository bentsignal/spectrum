#!/usr/bin/env python3
"""Validate Prism's exact CLT macOS SDK input and its complete tree seal."""

from __future__ import annotations

import hashlib
import os
import plistlib
import re
import stat
import subprocess
import sys
from pathlib import Path


SHA_PATTERN = re.compile(r"^[0-9a-f]{64}$")


def fail(message: str) -> "NoReturn":
    print(f"error: {message}", file=sys.stderr)
    raise SystemExit(1)


def file_sha(path: Path, root: Path) -> str:
    resolved = path.resolve()
    try:
        resolved.relative_to(root)
    except ValueError:
        fail(f"SDK key file escapes the SDK root: {path}")
    if not resolved.is_file() or resolved.is_symlink() or resolved.resolve() != resolved:
        fail(f"SDK key file target is missing, symlinked, or non-canonical: {path}")
    digest = hashlib.sha256()
    with resolved.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def verify_tree_ownership(root: Path, expected_uid: int, expected_gid: int) -> None:
    def verify(path: Path) -> None:
        metadata = path.lstat()
        relative = "." if path == root else str(path.relative_to(root))
        if metadata.st_uid != expected_uid or metadata.st_gid != expected_gid:
            fail(f"SDK node ownership mismatch: {relative}")
        if stat.S_IMODE(metadata.st_mode) & 0o022:
            fail(f"SDK node is group- or other-writable: {relative}")

    verify(root)
    for directory, names, files in os.walk(root, followlinks=False):
        parent = Path(directory)
        for name in names + files:
            verify(parent / name)


def main() -> None:
    if len(sys.argv) != 12:
        fail(
            f"usage: {Path(sys.argv[0]).name} <root> <canonical-name> <version> "
            "<tree-sha> <settings-sha> <libsystem-sha> <libcxx-sha> "
            "<tree-hasher> <expected-uid> <expected-gid> <contract-marker>"
        )
    root = Path(sys.argv[1])
    expected_name, expected_version = sys.argv[2:4]
    expected_tree, expected_settings, expected_libsystem, expected_libcxx = sys.argv[4:8]
    tree_hasher = Path(sys.argv[8])
    try:
        expected_uid = int(sys.argv[9])
        expected_gid = int(sys.argv[10])
    except ValueError:
        fail("SDK expected UID and GID must be non-negative integers")
    if expected_uid < 0 or expected_gid < 0:
        fail("SDK expected UID and GID must be non-negative integers")
    # The final reserved argument makes accidental older call sites fail closed.
    if sys.argv[11] != "prism-sdk-v1":
        fail("SDK validator contract marker mismatch")
    if not root.is_absolute() or not root.is_dir() or root.is_symlink() or root.resolve() != root:
        fail("SDK root is not an absolute canonical real directory")
    verify_tree_ownership(root, expected_uid, expected_gid)
    for name, value in {
        "tree": expected_tree,
        "settings": expected_settings,
        "libSystem": expected_libsystem,
        "libc++": expected_libcxx,
    }.items():
        if not SHA_PATTERN.fullmatch(value):
            fail(f"expected {name} SHA-256 is malformed")
    settings = root / "SDKSettings.plist"
    try:
        with settings.open("rb") as source:
            metadata = plistlib.load(source)
    except (OSError, plistlib.InvalidFileException) as error:
        fail(f"could not read SDKSettings.plist: {error}")
    if metadata.get("CanonicalName") != expected_name:
        fail("SDK canonical name mismatch")
    if metadata.get("Version") != expected_version:
        fail("SDK version mismatch")
    checks = {
        settings: expected_settings,
        root / "usr/lib/libSystem.tbd": expected_libsystem,
        root / "usr/lib/libc++.tbd": expected_libcxx,
    }
    for path, expected in checks.items():
        if file_sha(path, root) != expected:
            fail(f"SDK key file checksum mismatch: {path.relative_to(root)}")
    try:
        libsystem_header = (root / "usr/lib/libSystem.tbd").read_text(
            encoding="utf-8", errors="strict"
        ).split("install-name:", 1)[0]
    except (OSError, UnicodeError) as error:
        fail(f"could not inspect libSystem.tbd targets: {error}")
    if "targets:" not in libsystem_header or "arm64-macos" not in libsystem_header:
        fail("SDK root libSystem target does not include arm64-macos")
    try:
        tree = subprocess.run(
            [sys.executable, str(tree_hasher), str(root)],
            check=True,
            capture_output=True,
            text=True,
            timeout=120,
        ).stdout.strip()
    except (OSError, subprocess.SubprocessError) as error:
        fail(f"could not hash complete SDK tree: {error}")
    if tree != expected_tree:
        fail("complete SDK tree checksum mismatch")


if __name__ == "__main__":
    main()
