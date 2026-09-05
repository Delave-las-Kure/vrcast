#!/usr/bin/env bash
# The formulas the application carries over from the skill's scripts (constitution, VI).
#
# **Why this exists at all.** Principle VI says the hard-won rules MUST be carried from the
# existing scripts *without changing the formulas*, and that changing one MUST rest on a new
# measurement. Nothing checked that. On 2026-09-05 `convert.sh` had its bitrate cap corrected
# — rightly: it truncated the source to whole megabits before applying the ×1.6 allowance and
# again after, so a 7.842 Mbit/s HEVC master was capped at 11 where it is worth 12.5 — and the
# application went on computing 11. Two copies of one rule, a megabit apart, on nearly every
# real master. It was noticed by the isolation check, which is about something else entirely,
# and only because the same edit touched a protected file.
#
# **What it compares, and what it cannot.** Not the scripts against the core line by line: they
# are a shell script and a Rust module and always will be. What it holds is a short list of the
# exact expressions each side must still contain. A formula rewritten on either side loses its
# line, and this says so, naming both places.
#
# ⚠ **Run locally, not in continuous integration, and that is not an oversight.** The scripts
# live OUTSIDE the repository (principle VII); CI checks out the application alone, so there is
# nothing there to compare against. When the skill is absent this says the comparison was NOT
# MADE and exits 0 — it must never read as a pass, for the same reason
# `check-server-reference.sh` says so: a green step that compared nothing is worse than no step.
set -euo pipefail
export LC_ALL=C

APP="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ROOT="$(cd "$APP/.." && pwd)"
SCRIPTS="$ROOT/.claude/skills/vrcast-convert/scripts"

if [ ! -d "$SCRIPTS" ]; then
  echo "Скрипты скилла недоступны ($SCRIPTS) — сверка формул НЕ ВЫПОЛНЯЛАСЬ."
  echo "Это не успех: она идёт локально, там же, где лежат скиллы, а не в CI."
  exit 0
fi

fail=0
note() { printf '  %s\n' "$1"; }
bad() { printf 'РАСХОЖДЕНИЕ: %s\n' "$1"; fail=1; }

# Each rule: what it is, the script that owns it, the text that must still be in that script,
# the core file that carries it, and the text that must still be there.
#
# The texts are exact on purpose. A looser match — "the file mentions 1.6 somewhere" — would go
# on passing through the very rewrite this is here to catch.
check_rule() {
  local what="$1" script="$2" in_script="$3" core="$4" in_core="$5"

  if [ ! -f "$SCRIPTS/$script" ]; then
    bad "$what: скрипта $script нет вовсе"
    return
  fi
  local script_ok=0 core_ok=0
  grep -qF -- "$in_script" "$SCRIPTS/$script" && script_ok=1
  grep -qF -- "$in_core" "$APP/$core" && core_ok=1

  if [ "$script_ok" -eq 1 ] && [ "$core_ok" -eq 1 ]; then
    note "$what — сходится"
  elif [ "$script_ok" -eq 0 ] && [ "$core_ok" -eq 0 ]; then
    bad "$what: правило пропало С ОБЕИХ сторон. Искали «$in_script» в $script и «$in_core» в $core"
  elif [ "$script_ok" -eq 0 ]; then
    bad "$what: скрипт $script изменился, ядро осталось прежним. В скрипте больше нет «$in_script», в $core всё ещё «$in_core». Принцип VI: формула переносится из скрипта, а не расходится с ним"
  else
    bad "$what: ядро изменилось, скрипт остался прежним. В $core больше нет «$in_core», в $script всё ещё «$in_script». Изменение правила требует нового ЗАМЕРА, а не рассуждения"
  fi
}

echo "== формулы, перенесённые из скриптов (принцип VI) =="

# The heavier-codec allowance, and the units it is computed in. The units are half the rule:
# the ×1.6 was always there, and truncating the input to whole megabits before applying it is
# what quietly took twelve per cent off it.
check_rule "кап ×1.6 при HEVC-источнике" \
  convert.sh 'CAPK=$(( SRCBR_KBIT * 16 / 10 ))' \
  src-tauri/src/domain/ladder.rs 'kbit * 16 / 10'
check_rule "кап считается в килобитах, а не в целых мегабитах" \
  convert.sh 'SRCBR_KBIT=$(( SRCBR_BPS / 1000 ))' \
  src-tauri/src/domain/ladder.rs 'source.bitrate_bps / 1000'

# The ladder itself.
check_rule "множители ступеней" \
  plan-ladder.sh '(1.0,0.55,0.3,0.17)' \
  src-tauri/src/domain/ladder.rs '[1.0, 0.55, 0.3, 0.17]'
check_rule "постоянная 35, придавленная капом источника" \
  plan-ladder.sh 'ANCHOR=$(( SCAP < 35 ? SCAP : 35 ))' \
  src-tauri/src/domain/ladder.rs 'FALLBACK_MBPS: u64 = 35'
check_rule "якорь считается в целых мегабитах" \
  plan-ladder.sh 'ANCHOR=$(( psum / pn / 1000000 ))' \
  src-tauri/src/domain/ladder.rs 'measured_bps / MBIT'

# The encoder's ceiling.
check_rule "maxrate +10% от цели" \
  measure-ladder.sh 'MR=$(( BR * 11 / 10 ))' \
  src-tauri/src/domain/convert_plan.rs 'MAXRATE_PERCENT'

echo
if [ "$fail" -eq 0 ]; then
  echo "Формулы приложения и скриптов сходятся."
else
  echo "Формулы разошлись. Принцип VI: правила переносятся из скриптов без изменения, и"
  echo "изменение любого опирается на новый ЗАМЕР, а не на рассуждение."
  exit 1
fi
