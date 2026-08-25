#!/usr/bin/env bash
# T007 — the isolation check (constitution, principle VII).
#
# Developing VRCast Studio has no right to touch the running serving process. Everything new
# lives in vrcast-studio/; the skills, the reference, the serving configuration and the
# credentials stay untouched.
#
# What is checked is the ALLOW LIST below (PROTECTED) rather than "everything except
# vrcast-studio/": the root also holds large uploaded videos (uploads/), and hashing those on
# every check is too expensive. When a new protected file appears, add it to the list and
# rewrite the baseline. And remember: the baseline is rewritten by one command, so the order
# "check first, baseline afterwards" rests on discipline.
#
# Run from the project's root:
#   vrcast-studio/scripts/check-isolation.sh baseline   — record the baseline (once)
#   vrcast-studio/scripts/check-isolation.sh check      — compare (before a phase, in CI)
#
# The baseline is kept PER FILE rather than as one hash over a directory. That way the check
# firstly names the file that changed, and secondly does not depend on the order a directory
# is walked in: it used to, and gave a false alarm when the locale changed (caught
# 2026-08-24).
set -euo pipefail

# The locale is fixed: the sort order depends on it, and so does reproducibility.
export LC_ALL=C

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

PROTECTED=(
  ".claude/skills/vrcast-convert"
  ".claude/skills/vrcast-deploy"
  ".claude/skills/vrcast-diagnose"
  ".claude/skills/vrcast-hls"
  ".claude/skills/vrcast-upload"
  "vrcast-mtx"
  "SERVER.md"
  "server.env"
  "server.env.example"
  "server.txt"
)

STAMP="vrcast-studio/.isolation-baseline"

# Prints lines "<sha256>  <path>", one per file, in a stable order.
list_hashes() {
  local target
  for target in "${PROTECTED[@]}"; do
    if [ -d "$target" ]; then
      find "$target" -type f -print0 | sort -z | while IFS= read -r -d '' f; do
        printf '%s  %s\n' "$(sha256sum "$f" | cut -d' ' -f1)" "$f"
      done
    elif [ -f "$target" ]; then
      printf '%s  %s\n' "$(sha256sum "$target" | cut -d' ' -f1)" "$target"
    else
      printf '%s  %s\n' "MISSING" "$target"
    fi
  done | sort -k2
}

case "${1:-check}" in
  baseline)
    list_hashes > "$STAMP"
    echo "Baseline recorded: $STAMP ($(wc -l < "$STAMP") files)"
    ;;

  check)
    if [ ! -f "$STAMP" ]; then
      echo "ERROR: there is no baseline. First run: $0 baseline" >&2
      exit 2
    fi

    current="$(mktemp)"
    trap 'rm -f "$current"' EXIT
    list_hashes > "$current"

    if diff -q "$STAMP" "$current" >/dev/null; then
      echo "Isolation holds: not one of the $(wc -l < "$STAMP") protected files was changed."
      exit 0
    fi

    echo "ISOLATION BROKEN. Differences from the baseline:" >&2
    echo "" >&2
    # The baseline is the left column, the current state the right; only paths are shown.
    join -j 2 -v 1 -o '0' "$STAMP" "$current" 2>/dev/null | sed 's/^/  DELETED: /' >&2 || true
    join -j 2 -v 2 -o '0' "$STAMP" "$current" 2>/dev/null | sed 's/^/  ADDED:   /' >&2 || true
    join -j 2 -o '0,1.1,2.1' "$STAMP" "$current" 2>/dev/null \
      | awk '$2 != $3 { print "  CHANGED: " $1 }' >&2 || true

    echo "" >&2
    echo "The constitution's principle VII is broken: the working serving process was changed." >&2
    echo "Put the files back as they were — developing the application must not touch them." >&2
    exit 1
    ;;

  *)
    echo "Usage: $0 [baseline|check]" >&2
    exit 2
    ;;
esac
