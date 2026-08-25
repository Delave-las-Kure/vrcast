#!/usr/bin/env bash
# Проверка сборки под Linux с машины разработчика.
#
# Зачем. Код под `#[cfg(unix)]` — завершение дерева процессов, сигнал при смерти
# родителя, группы процессов — на Windows не компилируется вовсе. Любая ошибка в нём
# невидима, пока сборка не дойдёт до Linux. Первый же прогон в непрерывной интеграции
# поймал ровно это: лишний импорт, который под Windows негде было заметить.
#
# Проверка идёт в контейнере с тем же составом, что ставит CI. Первый запуск собирает
# образ и зависимости (несколько минут), дальше всё берётся из кеша.
#
# Запуск из каталога приложения:
#   bash scripts/check-linux.sh            # формат, clippy, сборка тестов
#   bash scripts/check-linux.sh test       # то же плюс прогон тестов ядра
set -euo pipefail

APP="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
IMAGE="vrcast-linux-check:1"
# Именованный том под каталог сборки: без него каждый запуск пересобирал бы
# все зависимости заново, и проверкой никто не пользовался бы.
VOLUME="vrcast-linux-target"
MODE="${1:-check}"

if ! docker info >/dev/null 2>&1; then
  echo "Docker не запущен. Откройте Docker Desktop и повторите." >&2
  exit 2
fi

# Образ собирается всегда: при неизменном Dockerfile это доли секунды из кеша,
# зато правка состава подхватывается сама.
docker build -q -t "$IMAGE" "$APP/scripts/linux-check" >/dev/null

case "$MODE" in
  check) CMD='cargo fmt --check && cargo clippy --all-targets --features integration -- -D warnings' ;;
  test)  CMD='cargo fmt --check && cargo clippy --all-targets --features integration -- -D warnings && cargo test' ;;
  *) echo "Использование: $0 [check|test]" >&2; exit 2 ;;
esac

echo "Проверка под Linux ($MODE)…"
# Путь для Docker переводится в вид с буквой диска. Git Bash подсовывает свои
# пути вида /f/…, Docker их не понимает и молча монтирует пустоту — а выглядит это
# как «в примонтированном каталоге нет Cargo.toml» на каталоге, где он заведомо есть.
# MSYS_NO_PATHCONV заодно не даёт переписать правую часть `:/src`.
SRC_MOUNT="$(cygpath -m "$APP/src-tauri" 2>/dev/null || echo "$APP/src-tauri")"

# Оболочка без `-l`: логин-оболочка перечитывает /etc/profile и затирает PATH,
# в котором лежит cargo. Ошибка выглядит как «cargo: command not found» на образе,
# где cargo заведомо есть.
MSYS_NO_PATHCONV=1 docker run --rm \
  -v "$SRC_MOUNT:/src" \
  -v "$VOLUME:/build/target" \
  "$IMAGE" \
  bash -c "$CMD"

echo "Под Linux собирается и проходит проверки стиля."
