#!/usr/bin/env bash
# Всё, что проверяет непрерывная интеграция, — одной командой перед отправкой.
#
# Зачем. Проверки разбросаны по четырём командам и двум языкам, и обойти их все
# по памяти получается не всегда: дважды подряд в CI уезжало то, что ловится
# за минуту на месте — сначала замечание clippy, добавленное после последнего
# прогона clippy, потом гонка в оснастке тестов. Каждый такой круг стоит десяти
# минут ожидания и одного лишнего коммита в истории.
#
# Порядок намеренный: сначала быстрое и дешёвое, потом долгое. Первая же неудача
# останавливает прогон — незачем ждать пять минут ради второй.
#
# Имена переменных и функций латиницей: в именах оболочка допускает только
# латиницу, цифры и подчёркивание, а кириллическое имя она даже не дочитывает
# до конца — падает на разборе. Русский остаётся там, где ему место: в тексте.
#
# Запуск из каталога приложения:
#   bash scripts/check-all.sh              # всё, кроме требующего Docker
#   bash scripts/check-all.sh --full       # плюс Linux и одноразовый сервер
set -uo pipefail

APP="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$APP"

FULL=0
[ "${1:-}" = "--full" ] && FULL=1

FAILED=""

summary() {
  printf '\n'
  if [ -z "$FAILED" ]; then
    printf '\033[32m--- всё проверено, можно отправлять ---\033[0m\n'
  else
    printf '\033[31m--- не прошло:%s ---\033[0m\n' "$FAILED"
    printf 'Отправлять нельзя: то же самое поймает CI, только через десять минут.\n'
  fi
}

step() {
  local name="$1"; shift
  printf '\n\033[1m> %s\033[0m\n' "$name"
  if "$@"; then
    printf '\033[32m  прошло: %s\033[0m\n' "$name"
  else
    printf '\033[31m  НЕ ПРОШЛО: %s\033[0m\n' "$name"
    FAILED="$FAILED «$name»"
    # Останавливаемся сразу: ждать ещё пять минут ради второй неудачи незачем.
    summary
    exit 1
  fi
}

step "Изоляция (принцип VII)" bash scripts/check-isolation.sh
step "Захардкоженные серверы (FR-004)" bash scripts/check-no-hardcoded-server.sh
step "Ядро: формат" cargo fmt --manifest-path src-tauri/Cargo.toml --check
step "Ядро: clippy со всеми целями" \
  cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --features integration -- -D warnings
step "Ядро: тесты" cargo test --manifest-path src-tauri/Cargo.toml
step "Интерфейс: типы" npm run --silent typecheck
step "Интерфейс: стиль" npm run --silent lint
step "Интерфейс: тесты" npm test --silent
step "Лицензии зависимостей ядра" \
  cargo deny --manifest-path src-tauri/Cargo.toml check licenses advisories
step "Перечень сторонних компонентов" npm run --silent check:third-party

if [ "$FULL" = "1" ]; then
  step "Сборка и тесты под Linux" bash scripts/check-linux.sh test
  step "Одноразовый сервер в контейнере" \
    cargo test --manifest-path src-tauri/Cargo.toml --features integration --test integration -- --test-threads=1
fi

summary
