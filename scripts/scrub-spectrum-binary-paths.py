#!/usr/bin/env python3
"""Atomically anonymize absolute build-root prefixes without resizing a binary."""

from __future__ import annotations

import os
import stat
import sys
import tempfile
from pathlib import Path


def fail(message: str) -> "NoReturn":
    print(f"error: {message}", file=sys.stderr)
    raise SystemExit(1)


def replacement_for(raw: bytes) -> bytes:
    prefix = b"/spectrum/"
    if len(raw) < len(prefix):
        fail("forbidden path is too short to scrub safely")
    return prefix + (b"_" * (len(raw) - len(prefix)))


def main() -> None:
    if len(sys.argv) < 3:
        fail(f"usage: {Path(sys.argv[0]).name} <binary> <absolute-prefix>...")
    binary = Path(sys.argv[1])
    if (
        not binary.is_absolute()
        or not binary.is_file()
        or binary.is_symlink()
        or binary.resolve() != binary
    ):
        fail("binary must be an absolute canonical regular non-symlink file")

    forbidden = sorted({os.fsencode(value) for value in sys.argv[2:]}, key=len, reverse=True)
    if any(not value.startswith(b"/") or b"\0" in value for value in forbidden):
        fail("every forbidden prefix must be an absolute path without NUL bytes")

    data = binary.read_bytes()
    replacements = 0
    for value in forbidden:
        replacement = replacement_for(value)
        count = data.count(value)
        data = data.replace(value, replacement)
        replacements += count
    if any(value in data for value in forbidden):
        fail("forbidden path remained after scrubbing")

    mode = stat.S_IMODE(binary.stat().st_mode)
    temporary_name = ""
    try:
        with tempfile.NamedTemporaryFile(dir=binary.parent, prefix=".path-scrub.", delete=False) as output:
            temporary_name = output.name
            output.write(data)
            output.flush()
            os.fsync(output.fileno())
        os.chmod(temporary_name, mode)
        os.replace(temporary_name, binary)
    finally:
        if temporary_name and os.path.exists(temporary_name):
            os.unlink(temporary_name)
    print(f"spectrum-ghostty-path-scrub replacements={replacements} binary={binary}")


if __name__ == "__main__":
    main()
