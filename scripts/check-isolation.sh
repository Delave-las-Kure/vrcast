#!/usr/bin/env bash
# T007 — проверка изоляции (конституция, принцип VII).
#
# Разработка VRCast Studio не имеет права трогать работающий процесс раздачи.
# Всё новое живёт в vrcast-studio/; скиллы, справочник, конфигурация раздачи и
# доступы остаются нетронутыми.
#
# Проверяется БЕЛЫЙ СПИСОК ниже (PROTECTED), а не «всё, кроме vrcast-studio/»:
# в корне лежат и большие загруженные видео (uploads/), хешировать которые при
# каждой проверке слишком дорого. Появился новый защищаемый файл — добавьте его
# в список и перепишите эталон. И помните: эталон переписывается одной командой,
# поэтому порядок «сначала check, потом baseline» держится на дисциплине.
#
# Запуск из корня проекта:
#   vrcast-studio/scripts/check-isolation.sh baseline   — записать эталон (один раз)
#   vrcast-studio/scripts/check-isolation.sh check      — сверить (перед сдачей фазы, в CI)
#
# Эталон хранится ПОФАЙЛОВО, а не одним хешем на каталог. Так проверка, во-первых,
# называет конкретный изменившийся файл, во-вторых, не зависит от порядка обхода
# каталога: раньше зависела и давала ложную тревогу при смене локали (поймано 2026-08-24).
set -euo pipefail

# Локаль фиксирована: от неё зависит порядок сортировки, а значит и воспроизводимость.
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

# Печатает строки "<sha256>  <путь>" по одной на файл, в устойчивом порядке.
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
      printf '%s  %s\n' "ОТСУТСТВУЕТ" "$target"
    fi
  done | sort -k2
}

case "${1:-check}" in
  baseline)
    list_hashes > "$STAMP"
    echo "Эталон записан: $STAMP ($(wc -l < "$STAMP") файлов)"
    ;;

  check)
    if [ ! -f "$STAMP" ]; then
      echo "ОШИБКА: нет эталона. Сначала: $0 baseline" >&2
      exit 2
    fi

    current="$(mktemp)"
    trap 'rm -f "$current"' EXIT
    list_hashes > "$current"

    if diff -q "$STAMP" "$current" >/dev/null; then
      echo "Изоляция соблюдена: ни один из $(wc -l < "$STAMP") защищённых файлов не изменён."
      exit 0
    fi

    echo "НАРУШЕНА ИЗОЛЯЦИЯ. Расхождения с эталоном:" >&2
    echo "" >&2
    # Левый столбец эталона, правый — текущее состояние; показываем только пути.
    join -j 2 -v 1 -o '0' "$STAMP" "$current" 2>/dev/null | sed 's/^/  УДАЛЁН:  /' >&2 || true
    join -j 2 -v 2 -o '0' "$STAMP" "$current" 2>/dev/null | sed 's/^/  ДОБАВЛЕН: /' >&2 || true
    join -j 2 -o '0,1.1,2.1' "$STAMP" "$current" 2>/dev/null \
      | awk '$2 != $3 { print "  ИЗМЕНЁН: " $1 }' >&2 || true

    echo "" >&2
    echo "Нарушен принцип VII конституции: рабочий процесс раздачи изменён." >&2
    echo "Верните файлы как были — разработка приложения не имеет права их трогать." >&2
    exit 1
    ;;

  *)
    echo "Использование: $0 [baseline|check]" >&2
    exit 2
    ;;
esac
