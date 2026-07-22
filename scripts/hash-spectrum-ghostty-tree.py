#!/usr/bin/env python3
"""Hash a filesystem tree including topology and permission metadata."""

from __future__ import annotations

import hashlib
import os
import stat
import sys
from pathlib import Path


def fail(message: str) -> "NoReturn":
    print(f"error: {message}", file=sys.stderr)
    raise SystemExit(1)


def encoded_record(kind: bytes, mode: int, relative: str, payload: bytes = b"") -> bytes:
    path = os.fsencode(relative)
    return b"".join(
        (
            kind,
            mode.to_bytes(4, "big"),
            len(path).to_bytes(8, "big"),
            path,
            len(payload).to_bytes(8, "big"),
            payload,
        )
    )


def hash_regular_file(path: Path, metadata: os.stat_result, relative: str) -> bytes:
    file_digest = hashlib.sha256()
    descriptor = os.open(path, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0))
    opened = os.fstat(descriptor)
    if (opened.st_dev, opened.st_ino) != (metadata.st_dev, metadata.st_ino):
        os.close(descriptor)
        fail(f"artifact changed while opening: {relative}")
    with os.fdopen(descriptor, "rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            file_digest.update(chunk)
        finished = os.fstat(source.fileno())
    if (
        finished.st_size != opened.st_size
        or finished.st_mtime_ns != opened.st_mtime_ns
        or stat.S_IMODE(finished.st_mode) != stat.S_IMODE(metadata.st_mode)
    ):
        fail(f"artifact changed while hashing: {relative}")
    return file_digest.digest()


def artifact_records(root: Path) -> list[tuple[bytes, bytes]]:
    records: list[tuple[bytes, bytes]] = []
    pending = [(root, ".")]
    while pending:
        path, relative = pending.pop()
        if "\n" in relative or "\r" in relative:
            fail(f"artifact path contains a newline: {relative!r}")
        metadata = path.lstat()
        mode = stat.S_IMODE(metadata.st_mode)
        sort_key = os.fsencode(relative)
        if stat.S_ISDIR(metadata.st_mode):
            records.append((sort_key, encoded_record(b"D", mode, relative)))
            children = sorted(path.iterdir(), key=lambda child: os.fsencode(child.name), reverse=True)
            for child in children:
                child_relative = child.name if relative == "." else f"{relative}/{child.name}"
                pending.append((child, child_relative))
        elif stat.S_ISREG(metadata.st_mode):
            digest = hash_regular_file(path, metadata, relative)
            payload = metadata.st_size.to_bytes(8, "big") + digest
            records.append((sort_key, encoded_record(b"F", mode, relative, payload)))
        else:
            fail(f"artifact tree contains a symlink or special node: {relative}")
    return sorted(records, key=lambda item: item[0])


def verify_overlay(source_root: Path, destination_root: Path) -> None:
    pending = [(source_root, ".")]
    while pending:
        source_path, item_relative = pending.pop()
        if "\n" in item_relative or "\r" in item_relative:
            fail(f"artifact path contains a newline: {item_relative!r}")
        source_metadata = source_path.lstat()
        destination_path = (
            destination_root
            if item_relative == "."
            else destination_root / item_relative
        )
        try:
            destination_metadata = destination_path.lstat()
        except FileNotFoundError:
            fail(f"overlay destination is missing artifact: {item_relative}")
        if (
            item_relative != "."
            and stat.S_IMODE(source_metadata.st_mode)
            != stat.S_IMODE(destination_metadata.st_mode)
        ):
            fail(f"overlay artifact mode differs: {item_relative}")
        if stat.S_ISDIR(source_metadata.st_mode):
            if not stat.S_ISDIR(destination_metadata.st_mode):
                fail(f"overlay artifact is not a directory: {item_relative}")
            children = sorted(
                source_path.iterdir(),
                key=lambda child: os.fsencode(child.name),
                reverse=True,
            )
            for child in children:
                child_relative = (
                    child.name
                    if item_relative == "."
                    else f"{item_relative}/{child.name}"
                )
                pending.append((child, child_relative))
        elif stat.S_ISREG(source_metadata.st_mode):
            if not stat.S_ISREG(destination_metadata.st_mode):
                fail(f"overlay artifact is not a regular file: {item_relative}")
            source_digest = hash_regular_file(
                source_path, source_metadata, item_relative
            )
            destination_digest = hash_regular_file(
                destination_path, destination_metadata, item_relative
            )
            if (
                source_metadata.st_size != destination_metadata.st_size
                or source_digest != destination_digest
            ):
                fail(f"overlay artifact content differs: {item_relative}")
        else:
            fail(f"artifact tree contains a symlink or special node: {item_relative}")


def main() -> None:
    if len(sys.argv) == 4 and sys.argv[1] == "--verify-overlay":
        source = Path(sys.argv[2])
        destination = Path(sys.argv[3])
        if not source.is_absolute() or not destination.is_absolute():
            fail("overlay paths must be absolute")
        verify_overlay(source, destination)
        return
    if len(sys.argv) != 2:
        fail(f"usage: {Path(sys.argv[0]).name} <artifact-tree> | --verify-overlay <source> <destination>")
    root = Path(sys.argv[1])
    if not root.is_absolute():
        fail("artifact tree path must be absolute")
    try:
        metadata = root.lstat()
    except FileNotFoundError:
        fail(f"artifact tree does not exist: {root}")
    if not stat.S_ISDIR(metadata.st_mode):
        fail(f"artifact tree root is not a real directory: {root}")
    digest = hashlib.sha256()
    for _, record in artifact_records(root):
        digest.update(len(record).to_bytes(8, "big"))
        digest.update(record)
    print(digest.hexdigest())


if __name__ == "__main__":
    main()
