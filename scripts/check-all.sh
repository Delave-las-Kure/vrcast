#!/usr/bin/env bash
# Everything continuous integration checks, in one command, before pushing.
#
# Why. The checks are spread over four commands and two languages, and going through them
# all from memory does not always work: twice in a row something reached CI that is caught
# in a minute here — first a clippy remark added after the last clippy run, then a race in
# the test fixture. Every such round costs ten minutes of waiting and one needless commit
# in the history.
#
# The order is deliberate: the quick and cheap first, the long afterwards. The first failure
# stops the run — there is no point waiting five minutes for a second one.
#
# WHAT THIS CHECK CANNOT SEE. It runs on a developer's machine, where everything is already
# in place: the bundled FFmpeg, the unpacked sources of dependencies. Continuous integration
# has none of that, and whatever depends on their presence is not checked here. That has
# already happened with the bundled programs: the Tauri bundler checks they are there, they
# were there locally, and the failure showed only in CI. So when the build is edited, use
# `--full`: it runs the same things in a clean Linux container where nothing is in place.
#
# Run from the application's directory:
#   bash scripts/check-all.sh              # everything that does not need Docker
#   bash scripts/check-all.sh --full       # plus Linux and the throwaway server
set -uo pipefail

APP="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$APP"

FULL=0
[ "${1:-}" = "--full" ] && FULL=1

FAILED=""
PARTLY=""

summary() {
  printf '\n'
  if [ -n "$PARTLY" ]; then
    printf '\033[33m--- ran without being able to check everything:%s ---\033[0m\n' "$PARTLY"
  fi
  if [ -z "$FAILED" ] && [ -n "$PARTLY" ]; then
    # Not "all checked": it was not. Safe to push all the same — what this machine could not
    # look at is exactly what continuous integration holds the secrets for.
    printf '\033[32m--- nothing failed here; the rest is for CI to look at ---\033[0m\n'
  elif [ -z "$FAILED" ]; then
    printf '\033[32m--- all checked, safe to push ---\033[0m\n'
  else
    printf '\033[31m--- failed:%s ---\033[0m\n' "$FAILED"
    printf 'Do not push: CI will catch the same thing, only ten minutes later.\n'
  fi
}

# ⚠ **Three answers, not two** (2026-09-05). "Passed" and "failed" were not enough, and the
# gap between them was being filled by the wrong one. Some checks here cannot always look at
# what they guard — the server's address and domain are secrets this machine legitimately does
# not have, and the skill's scripts live outside the repository — so they said so on standard
# error and exited zero. This runner reads only the exit code, so it printed "passed" over a
# check that had looked at nothing: the exact shape this project spends its days removing.
#
# Failing instead would be worse. A check that is red on every honest run teaches people not
# to read red, and that argument is already written into the leak guard. So the third answer,
# code 2: it ran, it could not see everything, and it says which. It does not stop a push —
# what it could not look at, continuous integration can.
step() {
  local name="$1"; shift
  printf '\n\033[1m> %s\033[0m\n' "$name"
  set +e
  "$@"
  local code=$?
  set -e
  if [ "$code" -eq 0 ]; then
    printf '\033[32m  passed: %s\033[0m\n' "$name"
  elif [ "$code" -eq 2 ]; then
    printf '\033[33m  partly: %s — see above for what was not looked at\033[0m\n' "$name"
    PARTLY="$PARTLY \"$name\""
  else
    printf '\033[31m  FAILED: %s\033[0m\n' "$name"
    FAILED="$FAILED \"$name\""
    # Stop at once: there is no point waiting another five minutes for a second failure.
    summary
    exit 1
  fi
}

step "Isolation (principle VII)" bash scripts/check-isolation.sh
step "Hardcoded servers (FR-004)" bash scripts/check-no-hardcoded-server.sh
# Local only, for the same reason as the isolation check: what it compares against lives
# outside the repository, and continuous integration checks out only the application.
step "The server reference and the resources agree" bash scripts/check-server-reference.sh
# Principle VI, and it had no guard until 2026-09-05: the formulas the core carries over
# from the skill's scripts. Beside the server reference because it is the same kind of
# check and has the same limit — the scripts live outside the repository, so this runs
# here and cannot run in CI.
step "The carried-over formulas agree" bash scripts/check-carried-rules.sh
step "Bundled FFmpeg can do what is needed" bash scripts/check-ffmpeg-features.sh
step "Core: format" cargo fmt --manifest-path src-tauri/Cargo.toml --check
step "Core: clippy over all targets" \
  cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --features integration -- -D warnings
step "Core: tests" cargo test --manifest-path src-tauri/Cargo.toml
# Windows only, and only where the NSIS compiler is: it arrives with a full bundle build.
# The script says so and passes rather than pretending — see its own note.
if [ "${OS:-}" = "Windows_NT" ]; then
  step "The uninstall hook's four cases"     powershell -NoProfile -ExecutionPolicy Bypass -File src-tauri/tests/uninstall-hook/run.ps1
fi
step "The workflow files" bash scripts/check-workflows.sh
step "The leak guard's rule" bash scripts/check-leak-guard.sh
step "The version is in one place" bash scripts/check-version.sh
# Not the release itself — the script that assembles it. It runs once, unwatched, at the
# one moment where a mistake publishes quietly instead of failing (T362).
step "The release file assembler" bash scripts/test-latest-json.sh
step "Both themes line up" bash scripts/check-theme.sh
step "Interface: types" npm run --silent typecheck
step "Interface: style" npm run --silent lint
step "Interface: tests" npm test --silent
step "Core dependencies' licences" \
  cargo deny --manifest-path src-tauri/Cargo.toml check licenses advisories
step "Third-party list" npm run --silent check:third-party
step "Interface dependencies' licences" npx --yes license-checker-rseidelsohn   --production --excludePrivatePackages --onlyAllow   "MIT;ISC;Apache-2.0;BSD-2-Clause;BSD-3-Clause;0BSD;CC0-1.0;Unlicense;Python-2.0;BlueOak-1.0.0"

if [ "$FULL" = "1" ]; then
  step "Build and tests on Linux" bash scripts/check-linux.sh test
  step "Throwaway server in a container" \
    cargo test --manifest-path src-tauri/Cargo.toml --features integration --test integration -- --test-threads=1
fi

summary
