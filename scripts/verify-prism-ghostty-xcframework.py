#!/usr/bin/env python3
"""Validate the exact macOS slice selected from GhosttyKit.xcframework."""

from __future__ import annotations

import plistlib
import sys
from pathlib import Path


def fail(message: str) -> "NoReturn":
    print(f"error: {message}", file=sys.stderr)
    raise SystemExit(1)


def main() -> None:
    if len(sys.argv) != 3:
        fail(f"usage: {Path(sys.argv[0]).name} <Info.plist> <library-identifier>")
    manifest = Path(sys.argv[1])
    target = sys.argv[2]
    try:
        with manifest.open("rb") as source:
            payload = plistlib.load(source)
    except (OSError, plistlib.InvalidFileException) as error:
        fail(f"invalid XCFramework manifest: {error}")
    libraries = payload.get("AvailableLibraries")
    if not isinstance(libraries, list):
        fail("XCFramework AvailableLibraries is not an array")
    matches = [
        entry
        for entry in libraries
        if isinstance(entry, dict) and entry.get("LibraryIdentifier") == target
    ]
    if len(matches) != 1:
        fail("selected XCFramework library identifier is missing or duplicated")
    entry = matches[0]
    if (
        entry.get("SupportedPlatform") != "macos"
        or "SupportedPlatformVariant" in entry
    ):
        fail("selected XCFramework library is not a native macOS slice")
    if (
        entry.get("LibraryPath") != "libghostty.a"
        or entry.get("BinaryPath") != "libghostty.a"
    ):
        fail("selected XCFramework library path is unexpected")
    if entry.get("HeadersPath") != "Headers":
        fail("selected XCFramework headers path is unexpected")
    architectures = entry.get("SupportedArchitectures")
    if (
        not isinstance(architectures, list)
        or len(architectures) != 2
        or set(architectures) != {"arm64", "x86_64"}
    ):
        fail("selected XCFramework architectures are not arm64 and x86_64")


if __name__ == "__main__":
    main()
