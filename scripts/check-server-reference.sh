#!/usr/bin/env bash
# T251 — the application's server resources against the skill's reference.
#
# Both deploy the same server, and they must go on deploying the same server. The skill stays
# the working fallback (constitution, principle VII): when the application is being rewritten,
# or will not start, or is being distrusted, the skill is what the serving is brought back
# with. Two references that have quietly drifted apart mean the fallback repairs a server the
# application does not recognise — and that is found out on the day it is needed.
#
# What this checks is not "the files are equal": three of them are deliberately different, and
# the differences are named below with their reasons. What it checks is that nothing ELSE has
# changed, and that what was bought with a mistake is still there, word for word.
#
# Run from the project's root:
#   vrcast-studio/scripts/check-server-reference.sh
set -euo pipefail
export LC_ALL=C

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

SKILL=".claude/skills/vrcast-deploy/reference"
RES="vrcast-studio/src-tauri/resources/server"

# The reference lives OUTSIDE the repository (principle VII: the skills are not part of the
# application), so on a machine where only the application was cloned there is nothing to
# compare against. That is said out loud and the exit code stays 0 — but it must never read
# as a pass, because a check that quietly reports success on a comparison it did not make is
# worse than no check at all. This is why it runs locally before a phase is handed over and
# not in continuous integration, which checks out the application alone.
if [ ! -d "$SKILL" ]; then
  echo "Эталон скилла недоступен ($SKILL) — сверка НЕ ВЫПОЛНЯЛАСЬ."
  echo "Это не успех: она идёт локально, там же, где лежат скиллы, а не в CI."
  exit 0
fi

fail=0
note() { printf '  %s\n' "$1"; }
bad() { printf 'РАСХОЖДЕНИЕ: %s\n' "$1"; fail=1; }

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

# ---------------------------------------------------------------- carried over word for word
#
# Both are pure measurement. The network one carries the reason BBR replaced cubic (cubic
# treats any packet loss as congestion, and on Wi-Fi and mobile the losses are random, so the
# speed collapsed for nothing) and the buffer sizes for fifteen viewers at 9-30 Mbit/s across
# half a country. The restart one exists because Caddy's stock unit has no Restart line at
# all, so a crash meant the serving lay down until somebody noticed.
echo "== дословный перенос =="
for f in 99-vrcast-net.conf caddy-10-restart.conf; do
  if [ ! -f "$RES/$f" ]; then
    bad "$f нет в ресурсах приложения"
  elif cmp -s "$SKILL/$f" "$RES/$f"; then
    note "$f — совпадает"
  else
    bad "$f разошёлся с эталоном скилла (см. diff «$SKILL/$f» «$RES/$f»)"
  fi
done

# ------------------------------------------------------------------- differs, and it is said
#
# The disk name. The skill writes vda in and warns to fix it by hand; the application has
# nobody to warn, so it works out the disk itself and substitutes. The value 8192 is the
# measured one -- the default 128 KB ran into virtio's latency at ~17 MB/s, and 8 MB gave
# 40-60 MB/s on sequential serving -- so the rule's line has to be identical apart from the
# disk.
echo "== отличается, и это записано =="
# The rule itself, not the comments that mention it: read_ahead_kb appears in both, and
# matching on it alone pulled in a comment line and compared prose.
skill_rule="$(grep '^SUBSYSTEM==' "$SKILL/60-vrcast-readahead.rules" || true)"
res_rule="$(grep '^SUBSYSTEM==' "$RES/60-vrcast-readahead.rules" || true)"
if [ -z "$skill_rule" ] || [ -z "$res_rule" ]; then
  bad "правило readahead не найдено в одном из файлов"
else
  # The only permitted difference is the disk itself. Blanked out with sed rather than with
  # bash's own substitution: the placeholder is {{DISK}}, and a pair of closing braces inside
  # ${...} ends the expansion early — the check then compared two unchanged strings and
  # reported a divergence that was not there.
  norm_skill="$(printf '%s' "$skill_rule" | sed 's/KERNEL=="[^"]*"/KERNEL==DISK/')"
  norm_res="$(printf '%s' "$res_rule" | sed 's/KERNEL=="[^"]*"/KERNEL==DISK/')"
  placeholders="$(printf '%s' "$res_rule" | grep -c '{{DISK}}' || true)"
  if [ "$placeholders" -ne 1 ]; then
    bad "в правиле приложения имя диска записано жёстко, а не подстановкой {{DISK}}: на не-virtio железе оно молча ни к чему не применится"
  elif [ "$norm_skill" = "$norm_res" ]; then
    note "60-vrcast-readahead.rules — совпадает, кроме имени диска (так и задумано)"
  else
    bad "правило readahead отличается не только именем диска:"
    note "скилл:      $skill_rule"
    note "приложение: $res_rule"
  fi
fi

# ----------------------------------------------------------------------------- the Caddyfile
#
# Two blocks are carried over word for word and are compared as such: the serving block (the
# blanket caching rule that a quality limit has to NARROW, the content types without which
# Caddy hands .ts out as text and .m3u8 as audio, Accept-Ranges, the cross-origin header) and
# the log block (its shape is what half of everything known about a viewer is read out of).
#
# A block is taken from the line that opens it to the first line that is a single tab and a
# closing brace -- the reference is formatted by `caddy fmt`, so that is exact rather than
# hopeful.
block() { # file, opening pattern -> stdout
  awk -v open="$2" '
    $0 ~ open { inside = 1 }
    inside { print }
    inside && $0 == "\t}" { exit }
  ' "$1"
}

echo "== Caddyfile =="
# The patterns are awk regexes: a brace and a star are written as character classes rather
# than backslash-escaped, because awk warns about the escapes and then treats them as the
# plain characters anyway — a warning on every run teaches people not to read warnings.
for pair in "handle_path /videos/[*]:блок раздачи" "log [{]:блок журнала"; do
  pattern="${pair%%:*}"
  title="${pair##*:}"
  block "$SKILL/Caddyfile" "$pattern" > "$work/skill.block"
  block "$RES/Caddyfile" "$pattern" > "$work/res.block"
  if [ ! -s "$work/skill.block" ] || [ ! -s "$work/res.block" ]; then
    bad "$title не найден в одном из файлов"
  elif cmp -s "$work/skill.block" "$work/res.block"; then
    note "$title — совпадает дословно"
  else
    bad "$title разошёлся:"
    diff "$work/skill.block" "$work/res.block" | sed 's/^/    /' || true
  fi
done

# The line the whole of milestone C hangs on. It is NOT in the skill's reference, and that is
# the difference this file records rather than hides: without it the rules file the
# application owns is imported by nothing, and a server deployed by the application would be
# one where the application's own quality limits do not work.
imports="$(grep -c 'import /etc/caddy/vrcast-limits.conf' "$RES/Caddyfile" || true)"
if [ "$imports" -eq 1 ]; then
  note "строка import на месте (её в эталоне скилла нет — так и задумано)"
else
  bad "строки import ровно один раз в Caddyfile приложения нет (найдено: $imports)"
fi

# ------------------------------------------------------------- absent on purpose (T252)
#
# MediaMTX is out of the server side by the owner's decision of 2026-08-27: the application
# never once went that way. This is checked rather than remembered, because re-adding it is a
# one-line change that looks harmless and quietly puts a service, a version to maintain and a
# source of failures in the health report back into version 1 of the server side.
echo "== чего быть не должно =="
for f in mediamtx.yml stream.sh mediamtx.service; do
  if [ -e "$RES/$f" ]; then
    bad "$f вернулся в ресурсы: MediaMTX убран решением T252, и возвращать его — пересматривать решение, а не добавлять файл"
  else
    note "$f отсутствует — верно"
  fi
done
proxies="$(grep -c 'reverse_proxy' "$RES/Caddyfile" || true)"
if [ "$proxies" -eq 0 ]; then
  note "проброса в Caddyfile нет — верно"
else
  bad "в Caddyfile приложения появился reverse_proxy ($proxies): вместе с ним возвращается и записанная ловушка про вечную петлю"
fi

echo
if [ "$fail" -eq 0 ]; then
  echo "Эталон и ресурсы сходятся."
else
  echo "Эталон и ресурсы разошлись. Приложение и скилл разворачивают разные серверы."
  exit 1
fi
