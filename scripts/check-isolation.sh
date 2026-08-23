#!/usr/bin/env bash
# T007 — проверка изоляции (конституция, принцип VII).
#
# Разработка VRCast Studio не имеет права трогать работающий процесс раздачи.
# Всё новое живёт в vrcast-studio/; скиллы, справочник и доступы остаются нетронутыми.
# Скрипт падает, если в корне проекта изменилось хоть что-то, кроме vrcast-studio/ и specs/.
#
# Запуск из корня проекта:  vrcast-studio/scripts/check-isolation.sh
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

PROTECTED=(
  ".claude/skills/vrcast-convert"
  ".claude/skills/vrcast-deploy"
  ".claude/skills/vrcast-diagnose"
  ".claude/skills/vrcast-hls"
  ".claude/skills/vrcast-upload"
  "SERVER.md"
  "server.env"
  "server.env.example"
  "server.txt"
)

STAMP="vrcast-studio/.isolation-baseline"
fail=0

hash_of() {
  # Устойчивый отпечаток пути: содержимое всех файлов + их имена.
  if [ -d "$1" ]; then
    find "$1" -type f -print0 | sort -z | xargs -0 sha256sum 2>/dev/null | sha256sum | cut -d' ' -f1
  elif [ -f "$1" ]; then
    sha256sum "$1" | cut -d' ' -f1
  else
    echo "MISSING"
  fi
}

case "${1:-check}" in
  baseline)
    : > "$STAMP"
    for p in "${PROTECTED[@]}"; do
      printf '%s  %s
' "$(hash_of "$p")" "$p" >> "$STAMP"
    done
    echo "Эталон записан: $STAMP ($(wc -l < "$STAMP") путей)"
    ;;
  check)
    if [ ! -f "$STAMP" ]; then
      echo "ОШИБКА: нет эталона. Сначала: $0 baseline" >&2
      exit 2
    fi
    while read -r want path; do
      got="$(hash_of "$path")"
      if [ "$got" = "MISSING" ] && [ "$want" != "MISSING" ]; then
        echo "УДАЛЁН вне vrcast-studio/: $path" >&2; fail=1
      elif [ "$got" != "$want" ]; then
        echo "ИЗМЕНЁН вне vrcast-studio/: $path" >&2; fail=1
      fi
    done < "$STAMP"

    if [ "$fail" -ne 0 ]; then
      echo "" >&2
      echo "Нарушен принцип VII конституции: рабочий процесс раздачи изменён." >&2
      echo "Верните файлы как были — разработка приложения не имеет права их трогать." >&2
      exit 1
    fi
    echo "Изоляция соблюдена: ничего вне vrcast-studio/ не изменено."
    ;;
  *)
    echo "Использование: $0 [baseline|check]" >&2; exit 2 ;;
esac
