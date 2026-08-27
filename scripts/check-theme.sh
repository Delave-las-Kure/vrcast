#!/usr/bin/env bash
# T325 — обе темы по-настоящему, а не одна с оговорками.
#
# Проверяется механически то, что механически проверяемо, и **только оно**:
#
# 1. у каждого цветового токена светлой темы есть пара в тёмной. Токен без пары — это цвет,
#    который в тёмной теме останется светлым, и заметят это на том экране, куда реже всего
#    заходят;
# 2. нигде, кроме двух блоков с токенами, нет цвета впрямую. Один `#fff`, поставленный на
#    экране, который смотрели, — это обычный способ выпустить нечитаемую тему: он не
#    переключается вместе со всем остальным и виден только там, куда не заглянули.
#
# Читаемость глазами этим не проверяется и не притворяется проверенной: это сценарий 9
# quickstart, и его проходит человек.
set -euo pipefail

CSS="$(dirname "$0")/../src/app/styles.css"
RED=$'\033[31m'; GREEN=$'\033[32m'; OFF=$'\033[0m'
fail=0

python - "$CSS" <<'PY'
import re
import sys

path = sys.argv[1]
text = open(path, encoding="utf-8").read()


def block(selector):
    """Тело правила по его заголовку."""
    i = text.index(selector + " {")
    depth = 0
    for j in range(i, len(text)):
        if text[j] == "{":
            depth += 1
        elif text[j] == "}":
            depth -= 1
            if depth == 0:
                return text[i:j]
    raise SystemExit("не закрыт блок " + selector)


light = block(":root")
dark = block(':root[data-theme="dark"]')

TOKEN = re.compile(r"^\s*(--[a-z0-9-]+):\s*([^;]+);", re.M)
COLOUR = re.compile(r"#[0-9a-fA-F]{3,8}\b|\brgba?\(|\bhsla?\(")

light_tokens = {m.group(1): m.group(2).strip() for m in TOKEN.finditer(light)}
dark_tokens = {m.group(1): m.group(2).strip() for m in TOKEN.finditer(dark)}

problems = []

# 1. Каждый цветовой токен светлой темы имеет пару в тёмной.
for name, value in light_tokens.items():
    if not COLOUR.search(value):
        continue  # не цвет: радиусы, скорости, кривые
    if name not in dark_tokens:
        problems.append("токен %s есть в светлой теме и отсутствует в тёмной" % name)

# 2. Ни одного цвета впрямую вне блоков с токенами.
rest = text.replace(light, "").replace(dark, "")
for n, line in enumerate(rest.splitlines(), 1):
    stripped = line.strip()
    if stripped.startswith("*") or stripped.startswith("/*") or stripped.startswith("//"):
        continue
    if COLOUR.search(line):
        problems.append("цвет впрямую вне токенов: %s" % stripped)

if problems:
    for p in problems:
        print("  " + p)
    sys.exit(1)

print("  токенов в светлой: %d, из них цветовых с парой в тёмной: %d" % (
    len(light_tokens),
    sum(1 for n, v in light_tokens.items() if COLOUR.search(v)),
))
PY
status=$?

if [[ $status -ne 0 ]]; then
	echo "${RED}--- обе темы: НЕ сходятся ---${OFF}" >&2
	fail=1
else
	echo "${GREEN}--- обе темы: сходятся ---${OFF}"
fi

exit $fail
