#!/usr/bin/env bash
# T243 — the bundled FFmpeg must be able to do the things the application depends on.
#
# **Two of them, and neither is optional:**
#
#   libx264  — the software encoder. Without it a machine with no suitable graphics card has
#              nothing to prepare a file with at all (FR-026).
#   libvmaf  — measuring quality. A ladder is chosen by measuring the material, not by a
#              formula (R-21), so without this there is no ladder — and finding that out on
#              a person's machine, half an hour into a job they agreed to, is finding it out
#              far too late.
#
# The build is pinned by scripts/ffmpeg.json, so this cannot change by itself — but it can
# change when somebody moves the pin, and that is exactly the moment to notice.
set -uo pipefail

cd "$(dirname "$0")/.." || exit 1

case "$(uname -s)" in
  MINGW* | MSYS* | CYGWIN*) TRIPLE="x86_64-pc-windows-msvc"; SUFFIX=".exe" ;;
  Darwin)                   TRIPLE="x86_64-apple-darwin";    SUFFIX="" ;;
  *)                        TRIPLE="x86_64-unknown-linux-gnu"; SUFFIX="" ;;
esac

FFMPEG="src-tauri/binaries/ffmpeg-${TRIPLE}${SUFFIX}"
if [ ! -f "$FFMPEG" ]; then
  echo "no bundled FFmpeg at $FFMPEG — run 'npm run ffmpeg' first" >&2
  exit 1
fi

fail=0

# The encoder is looked for in the name column of the listing, not anywhere in the text:
# "x264" also stands inside "libx264rgb" and in half the descriptions.
if "$FFMPEG" -hide_banner -encoders 2>/dev/null | awk '{ print $2 }' | grep -qx "libx264"; then
  echo "  libx264: yes"
else
  echo "  libx264: MISSING — a machine without a graphics card could prepare nothing" >&2
  fail=1
fi

# Likewise for the filter: "libvmaf" also appears in the description of "vmafmotion", which
# is a different filter measuring a different thing.
if "$FFMPEG" -hide_banner -filters 2>/dev/null | awk '{ print $2 }' | grep -qx "libvmaf"; then
  echo "  libvmaf: yes"
else
  echo "  libvmaf: MISSING — quality cannot be measured, so no ladder can be built" >&2
  fail=1
fi

if [ "$fail" -ne 0 ]; then
  echo "the bundled FFmpeg cannot do what the application needs of it" >&2
  exit 1
fi
echo "the bundled FFmpeg can do what the application needs of it"
