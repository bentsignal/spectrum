#!/usr/bin/env python3
"""Run a noninteractive packaging command with a process-group timeout."""

from __future__ import annotations

import os
import signal
import subprocess
import sys
import time
from pathlib import Path


def fail(message: str, code: int = 2) -> "NoReturn":
    print(f"error: {message}", file=sys.stderr)
    raise SystemExit(code)


def process_group_exists(pgid: int, process: subprocess.Popen[bytes]) -> bool:
    # Reap the direct child when possible so its zombie cannot make a dead
    # process group appear live. Descendants remain in the session's PGID.
    process.poll()
    try:
        os.killpg(pgid, 0)
    except ProcessLookupError:
        return False
    except PermissionError:
        return True
    return True


def wait_for_process_group_exit(
    pgid: int, process: subprocess.Popen[bytes], seconds: float
) -> bool:
    deadline = time.monotonic() + seconds
    while process_group_exists(pgid, process):
        if time.monotonic() >= deadline:
            return False
        time.sleep(0.05)
    return True


def write_live_marker(marker: Path, pgid: int) -> None:
    parent = marker.parent
    if not marker.is_absolute() or not parent.is_dir() or parent.is_symlink():
        fail(f"live-process marker parent is not an absolute real directory: {parent}")
    if parent.resolve() != parent:
        fail(f"live-process marker parent is non-canonical: {parent}")
    temporary = parent / f".{marker.name}.{os.getpid()}.tmp"
    descriptor = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    try:
        with os.fdopen(descriptor, "w", encoding="utf-8", newline="\n") as output:
            output.write(f"FORMAT=1\nPGID={pgid}\n")
            output.flush()
            os.fsync(output.fileno())
        os.replace(temporary, marker)
    finally:
        try:
            temporary.unlink()
        except FileNotFoundError:
            pass


def main() -> None:
    if len(sys.argv) < 5 or sys.argv[3] != "--":
        fail(
            f"usage: {os.path.basename(sys.argv[0])} "
            "<seconds> <absolute-live-marker> -- <command> [args ...]"
        )
    try:
        timeout = float(sys.argv[1])
    except ValueError:
        fail("timeout must be a positive number")
    if timeout <= 0:
        fail("timeout must be a positive number")

    marker = Path(sys.argv[2])
    if not marker.is_absolute() or not marker.parent.is_dir() or marker.parent.is_symlink():
        fail("live-process marker parent must be an absolute real directory")
    if marker.parent.resolve() != marker.parent:
        fail("live-process marker parent must be canonical")
    if marker.exists() or marker.is_symlink():
        fail(f"live-process marker already exists; refusing to launch: {marker}")

    command = sys.argv[4:]
    try:
        process = subprocess.Popen(
            command,
            stdin=subprocess.DEVNULL,
            start_new_session=True,
        )
    except OSError as error:
        fail(f"could not execute {command[0]}: {error}", 1)
    pgid = process.pid
    deadline = time.monotonic() + timeout
    while True:
        exit_code = process.poll()
        if exit_code is not None and not process_group_exists(pgid, process):
            raise SystemExit(exit_code)
        if time.monotonic() >= deadline:
            break
        time.sleep(0.05)

    print(
        f"error: command timed out after {timeout:g} seconds: {command[0]}",
        file=sys.stderr,
    )
    try:
        os.killpg(pgid, signal.SIGTERM)
    except ProcessLookupError:
        pass
    if not wait_for_process_group_exit(pgid, process, 5):
        try:
            os.killpg(pgid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        if not wait_for_process_group_exit(pgid, process, 5):
            write_live_marker(marker, pgid)
            print(
                "error: timed-out process group remained live after SIGKILL; "
                f"retaining its private root via {marker}",
                file=sys.stderr,
            )
    raise SystemExit(124)


if __name__ == "__main__":
    main()
