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
#
# **The listings are taken into variables and searched there, not piped into `grep -q`.**
# The first version of this did pipe them, and on continuous integration it declared libvmaf
# missing from a build that has it — proved by running the very same file in a container.
# The likeliest reason is that `grep -q` leaves as soon as it matches, the writer upstream
# dies of a broken pipe, and `pipefail` then reports the whole pipeline as failed although
# the thing was found. It was not reproduced, and that is exactly why the pipeline is gone
# rather than adjusted: a check that reports a missing feature when the feature is there is
# worse than no check, because it is believed.
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

ENCODERS="$("$FFMPEG" -hide_banner -encoders 2>/dev/null)"
FILTERS="$("$FFMPEG" -hide_banner -filters 2>/dev/null)"

if [ -z "$ENCODERS" ] || [ -z "$FILTERS" ]; then
  echo "the bundled FFmpeg at $FFMPEG would not say what it can do" >&2
  echo "  encoders listing: ${#ENCODERS} characters" >&2
  echo "  filters listing:  ${#FILTERS} characters" >&2
  exit 1
fi

fail=0

# Only the name column is looked at, never the whole line. "x264" also stands inside
# "libx264rgb" and in half the descriptions; "libvmaf" stands in the description of
# "vmafmotion", which is a different filter measuring a different thing.
has_name() {
  echo "$1" | awk -v want="$2" '$2 == want { found = 1 } END { exit found ? 0 : 1 }'
}

if has_name "$ENCODERS" libx264; then
  echo "  libx264: yes"
else
  echo "  libx264: MISSING — a machine without a graphics card could prepare nothing" >&2
  echo "    (the encoders listing was ${#ENCODERS} characters long)" >&2
  fail=1
fi

if has_name "$FILTERS" libvmaf; then
  echo "  libvmaf: yes"
else
  echo "  libvmaf: MISSING — quality cannot be measured, so no ladder can be built" >&2
  echo "    (the filters listing was ${#FILTERS} characters long)" >&2
  fail=1
fi

if [ "$fail" -ne 0 ]; then
  echo "the bundled FFmpeg cannot do what the application needs of it" >&2
  exit 1
fi
echo "the bundled FFmpeg can do what the application needs of it"
