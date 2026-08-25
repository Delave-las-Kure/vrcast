#!/usr/bin/env bash
# T064 — поиск секретов в журнале (SC-011, конституция, принцип IV).
#
# Прогоняет приёмочную проверку на настоящем сервере с самым подробным журналом,
# какой умеет приложение, и ищет в выводе следы приватного ключа. Смысл именно
# в подробном журнале: на уровне info разговорчивые библиотеки молчат, и проверка
# была бы вхолостую — а на trace они выкладывают всё, что знают, включая то, что
# знать никому не следует.
#
# Скрипт НИКОГДА не печатает содержимое ключа: он берёт из него отрезок, ищет его
# в журнале и сообщает только «нашлось» или «не нашлось».
#
# Запуск (нужен настоящий сервер и server.env рядом):
#   bash scripts/check-no-secrets-in-logs.sh
set -euo pipefail

APP="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ROOT="$(cd "$APP/.." && pwd)"
ENV_FILE="$ROOT/server.env"

if [ ! -f "$ENV_FILE" ]; then
  echo "Нет $ENV_FILE — проверка рассчитана на машину, где настроен настоящий сервер." >&2
  exit 2
fi

KEY=$(grep -E "^SSH_KEY=" "$ENV_FILE" | sed -E 's/^SSH_KEY="?([^"#]*)"?.*/\1/' | sed "s|\$HOME|$HOME|" | xargs)
if [ ! -f "$KEY" ]; then
  echo "Файл ключа из server.env не найден." >&2
  exit 2
fi

LOG="$(mktemp)"
trap 'rm -f "$LOG" "$LOG.needles"' EXIT

echo "Прогон приёмочной проверки с подробным журналом…"
(
  cd "$APP/src-tauri"
  VRCAST_LOG=trace cargo test --features integration --test integration \
    -- --ignored --nocapture живой_сервер
) > "$LOG" 2>&1 || {
  echo "Приёмочная проверка не прошла — искать секреты в журнале неудавшегося прогона рано." >&2
  tail -20 "$LOG" >&2
  exit 1
}

# Отрезки, которых в журнале быть не должно. Тело ключа берём из середины файла:
# первая строка у всех ключей одинакова и ничего не доказывает.
{
  sed -n '3,6p' "$KEY"
  echo "BEGIN OPENSSH PRIVATE KEY"
  echo "BEGIN RSA PRIVATE KEY"
} > "$LOG.needles"

found=0
while IFS= read -r needle; do
  [ ${#needle} -lt 12 ] && continue
  if grep -qF "$needle" "$LOG"; then
    echo "НАЙДЕН СЛЕД СЕКРЕТА В ЖУРНАЛЕ (отрезок длиной ${#needle} знаков)" >&2
    found=1
  fi
done < "$LOG.needles"

# Заодно убеждаемся, что журнал вообще писался: пустой журнал «прошёл бы» проверку,
# ничего не проверив.
lines=$(wc -l < "$LOG")
if [ "$lines" -lt 50 ]; then
  echo "Журнал подозрительно короткий ($lines строк) — проверка, скорее всего, вхолостую." >&2
  exit 1
fi

if [ "$found" -ne 0 ]; then
  echo "" >&2
  echo "Нарушен принцип IV: секрет попал в журнал." >&2
  exit 1
fi

echo "Секретов в журнале не найдено ($lines строк проверено, уровень trace)."
