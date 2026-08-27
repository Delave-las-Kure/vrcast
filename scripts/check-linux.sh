#!/usr/bin/env bash
# Checking the Linux build from a developer's machine.
#
# Why. Code under `#[cfg(unix)]` — terminating a process tree, the signal on a parent's
# death, process groups — does not compile on Windows at all. Any mistake in it is invisible
# until the build reaches Linux. The very first run in continuous integration caught exactly
# that: a needless import there was nowhere to notice under Windows.
#
# The check runs in a container with the same contents CI installs. The first run builds the
# image and the dependencies (a few minutes), after which everything comes from the cache.
#
# Run from the application's directory:
#   bash scripts/check-linux.sh            # format, clippy AND A TEST RUN
#   bash scripts/check-linux.sh style      # format and clippy only, faster
#
# The tests run by default deliberately. The very first attempt to do without them missed a
# real fault: sweeping up surviving programs did not work precisely under Linux, and only
# continuous integration saw it. A check that does not run what breaks saves minutes and
# costs hours.
set -euo pipefail

APP="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
IMAGE="vrcast-linux-check:3"
# A named volume for the build directory: without one every run would rebuild all the
# dependencies from scratch, and nobody would use the check.
VOLUME="vrcast-linux-target"
MODE="${1:-test}"

if ! docker info >/dev/null 2>&1; then
  echo "Docker is not running. Open Docker Desktop and try again." >&2
  exit 2
fi

# The image is always built: with the Dockerfile unchanged that is a fraction of a second
# from the cache, while an edit to its contents is picked up by itself.
docker build -q -t "$IMAGE" "$APP/scripts/linux-check" >/dev/null

STYLE='cargo fmt --check && cargo clippy --all-targets --features integration -- -D warnings'
case "$MODE" in
  test)  CMD="$STYLE && cargo test" ;;
  style) CMD="$STYLE" ;;
  *) echo "Usage: $0 [test|style]" >&2; exit 2 ;;
esac

# The bundled FFmpeg for Linux is put in place BEFOREHAND, from this machine: the Tauri
# bundler checks that the bundled programs are there and refuses to build without them, and
# the container has no Node to fetch them with. Without this step the check would fail over
# nothing — and that is exactly what already happened in CI, where the bundled programs were
# missing too.
echo "Preparing the bundled FFmpeg for Linux…"
node "$APP/scripts/fetch-ffmpeg.mjs" --for linux-x64

echo "Checking on Linux ($MODE)…"
# THE WHOLE APPLICATION is mounted, not just the core. The contract tests read the
# interface's type declarations one floor up (src/shared/contract.ts) and compare them
# against the core both ways; with src-tauri alone mounted they failed on the missing file —
# the check complained about code that was fine.
#
# The path is converted to the drive-letter form for Docker: Git Bash offers paths of its own
# like /f/…, Docker does not understand them and quietly mounts nothing. It looks like "there
# is no Cargo.toml" in a directory that certainly holds one. MSYS_NO_PATHCONV also stops the
# right-hand side of `:/app` being rewritten.
APP_MOUNT="$(cygpath -m "$APP" 2>/dev/null || echo "$APP")"

# A shell without `-l`: a login shell re-reads /etc/profile and wipes out the PATH that holds
# cargo. The failure looks like "cargo: command not found" on an image that certainly has it.
MSYS_NO_PATHCONV=1 docker run --rm \
  -v "$APP_MOUNT:/app" \
  -v "$VOLUME:/build/target" \
  -w /app/src-tauri \
  "$IMAGE" \
  bash -c "$CMD"

echo "It builds on Linux and passes the style checks."
