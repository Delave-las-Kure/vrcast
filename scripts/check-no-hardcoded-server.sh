#!/usr/bin/env bash
# T060a — в приложении не должно быть ни одного захардкоженного сервера (FR-004).
#
# Приложение раздаётся людям, у каждого свой сервер. Адрес, домен или путь автора,
# случайно занесённый при переносе логики из скиллов, — это дефект, который у чужого
# пользователя проявится молча.
set -euo pipefail

APP="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$APP"

# Значения берутся из окружения, чтобы сам скрипт не стал местом хранения адреса.
# В CI задаются секретами; локально — из server.env вручную.
NEEDLES=()
[ -n "${FORBID_IP:-}" ]     && NEEDLES+=("$FORBID_IP")
[ -n "${FORBID_DOMAIN:-}" ] && NEEDLES+=("$FORBID_DOMAIN")
NEEDLES+=("/var/lib/vrcast/videos")   # путь по умолчанию допустим ТОЛЬКО как значение по умолчанию в настройках

if [ "${#NEEDLES[@]}" -eq 0 ]; then
  echo "Предупреждение: FORBID_IP и FORBID_DOMAIN не заданы — проверка неполная." >&2
fi

fail=0
for needle in "${NEEDLES[@]}"; do
  hits="$(grep -rInF "$needle" src src-tauri/src 2>/dev/null || true)"
  if [ -n "$hits" ]; then
    echo "НАЙДЕН захардкоженный сервер ($needle):" >&2
    echo "$hits" >&2
    fail=1
  fi
done

if [ "$fail" -ne 0 ]; then
  echo "" >&2
  echo "Нарушен FR-004. Адрес сервера обязан приходить только из профиля пользователя." >&2
  exit 1
fi
echo "Захардкоженных серверов не найдено."
