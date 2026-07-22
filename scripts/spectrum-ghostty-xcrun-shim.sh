#!/usr/bin/env bash
set -euo pipefail

if [[ $# -eq 3 && "$1" == "--sdk" && "$2" == "macosx" \
  && "$3" == "--show-sdk-path" ]]; then
  sdk_root="${SPECTRUM_GHOSTTY_MACOS_SDK_ROOT:-}"
  [[ -n "$sdk_root" && "$sdk_root" == /* && -d "$sdk_root" && ! -L "$sdk_root" \
    && "$(realpath "$sdk_root")" == "$sdk_root" ]] || {
    echo "error: SPECTRUM_GHOSTTY_MACOS_SDK_ROOT is not an absolute canonical real directory" >&2
    exit 1
  }
  printf '%s\n' "$sdk_root"
  exit 0
fi

exec /usr/bin/xcrun "$@"
