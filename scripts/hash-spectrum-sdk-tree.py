#!/usr/bin/env python3
"""Hash a pinned SDK tree including ownership, topology, modes, and bytes."""

from __future__ import annotations

import hashlib
import os
import stat
import sys
from pathlib import Path, PurePosixPath


def fail(message: str) -> "NoReturn":
    print(f"error: {message}", file=sys.stderr)
    raise SystemExit(1)


def field(digest: "hashlib._Hash", value: str) -> None:
    encoded = value.encode("utf-8", errors="strict")
    digest.update(len(encoded).to_bytes(8, "big"))
    digest.update(encoded)


def safe_link_target(relative: str, target: str) -> bool:
    candidate = PurePosixPath(relative).parent / PurePosixPath(target)
    depth = 0
    for part in candidate.parts:
        if part in {"", "."}:
            continue
        if part == "..":
            depth -= 1
            if depth < 0:
                return False
        else:
            depth += 1
    return True


def hash_tree(root: Path) -> str:
    if not root.is_absolute() or not root.is_dir() or root.is_symlink():
        fail(f"SDK root is not an absolute real directory: {root}")
    if root.resolve() != root:
        fail(f"SDK root has a symlinked or non-canonical component: {root}")

    digest = hashlib.sha256(b"spectrum-sdk-tree-v1\0")
    root_metadata = root.lstat()
    root_mode = stat.S_IMODE(root_metadata.st_mode)
    field(digest, f"root:{root_mode:o}:{root_metadata.st_uid}:{root_metadata.st_gid}")

    def visit(directory: Path, prefix: str) -> None:
        try:
            entries = sorted(os.scandir(directory), key=lambda entry: os.fsencode(entry.name))
        except OSError as error:
            fail(f"could not scan SDK directory {directory}: {error}")
        for entry in entries:
            relative = f"{prefix}/{entry.name}" if prefix else entry.name
            if "\n" in relative or "\r" in relative:
                fail(f"SDK path contains a newline: {relative!r}")
            path = Path(entry.path)
            metadata = path.lstat()
            mode = stat.S_IMODE(metadata.st_mode)
            if stat.S_ISLNK(metadata.st_mode):
                target = os.readlink(path)
                if not target or target.startswith("/") or "\n" in target or "\r" in target:
                    fail(f"SDK symlink has an unsafe target: {relative}")
                if not safe_link_target(relative, target):
                    fail(f"SDK symlink escapes the root: {relative}")
                field(digest, "link")
                field(digest, relative)
                field(digest, f"{mode:o}")
                field(digest, str(metadata.st_uid))
                field(digest, str(metadata.st_gid))
                field(digest, target)
            elif stat.S_ISDIR(metadata.st_mode):
                field(digest, "directory")
                field(digest, relative)
                field(digest, f"{mode:o}")
                field(digest, str(metadata.st_uid))
                field(digest, str(metadata.st_gid))
                visit(path, relative)
            elif stat.S_ISREG(metadata.st_mode):
                field(digest, "file")
                field(digest, relative)
                field(digest, f"{mode:o}")
                field(digest, str(metadata.st_uid))
                field(digest, str(metadata.st_gid))
                field(digest, str(metadata.st_size))
                with path.open("rb") as source:
                    for chunk in iter(lambda: source.read(1024 * 1024), b""):
                        digest.update(chunk)
            else:
                fail(f"SDK contains a special node: {relative}")

    visit(root, "")
    return digest.hexdigest()


def main() -> None:
    if len(sys.argv) != 2:
        fail(f"usage: {Path(sys.argv[0]).name} <absolute-sdk-root>")
    print(hash_tree(Path(sys.argv[1])))


if __name__ == "__main__":
    main()
