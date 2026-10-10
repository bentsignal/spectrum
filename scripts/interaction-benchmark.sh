#!/usr/bin/env bash
# Runs the desktop app's interaction benchmark against strict budgets.
#
#   bash scripts/interaction-benchmark.sh [--profile interactive|software|ci]
#                                         [--report <file>] [--photo <file>]
#
# The app makes a throwaway library with large photos (one fully edited) and
# canvases from one photo to twelve,
# drives image and canvas editing through its real handlers, and exits
# nonzero when an edit is slow to reach the screen, the main thread stalls,
# the canvas is slow to settle, or an error is shown. On Linux it runs on a
# private X display with software rendering (the `software` profile); on
# macOS it opens a window on the desktop (the `interactive` profile).
set -euo pipefail
repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"
app="${SPECTRUM_DESKTOP:-target/release/spectrum-desktop}"
if [[ -z "${SPECTRUM_DESKTOP:-}" ]]; then
  cargo build --release --locked -p spectrum-desktop
fi

case "$(uname -s)" in
  Linux)
    profile=software
    display=":$((300 + RANDOM % 600))"
    Xvfb "$display" -screen 0 1600x1000x24 >/dev/null 2>&1 &
    xvfb=$!
    trap 'kill "$xvfb" 2>/dev/null || true' EXIT
    sleep 1
    export DISPLAY="$display"
    unset WAYLAND_DISPLAY WAYLAND_SOCKET
    # Software Vulkan; hardware drivers cannot present into Xvfb.
    for icd in /run/opengl-driver/share/vulkan/icd.d/lvp_icd.*.json \
      /usr/share/vulkan/icd.d/lvp_icd.*.json; do
      if [[ -e "$icd" ]]; then
        export VK_ICD_FILENAMES="$icd"
        break
      fi
    done
    ;;
  *) profile=interactive ;;
esac

# An explicit --profile replaces the default for this system.
for argument in "$@"; do
  if [[ "$argument" == --profile ]]; then
    profile=""
  fi
done
exec "$app" --benchmark --strict ${profile:+--profile "$profile"} "$@"
