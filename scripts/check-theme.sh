#!/usr/bin/env bash
# T325 — both themes for real, not one of them with excuses.
#
# What is checked is what can be checked by machine, and **only** that:
#
# 1. every colour token of the light theme has a counterpart in the dark one. A token without
#    one is a colour that stays light in the dark theme, and it will be noticed on the screen
#    people visit least often;
# 2. nowhere outside the two token blocks is a colour written out directly. One `#fff` put on
#    the screen somebody happened to be looking at is the ordinary way to ship an unreadable
#    theme: it does not switch with the rest, and it is visible only where nobody looked.
#
# Readability by eye is not checked here and does not pretend to be: that is scenario 9 of the
# quickstart, and a person walks it.
set -euo pipefail

CSS="$(dirname "$0")/../src/app/styles.css"

# **`python3`, then `python`.** Ubuntu ships only the first, Windows installers only the
# second, and a script that names one of them works on one machine of the two. Found on the
# self-hosted runner's first run: `python: command not found` on Ubuntu 22.04.
PY_BIN=$(command -v python3 || command -v python || true)
if [ -z "$PY_BIN" ]; then
	echo "${RED}--- the two themes: NOT CHECKED (no python on this machine) ---${OFF}" >&2
	exit 1
fi
RED=$'\033[31m'; GREEN=$'\033[32m'; OFF=$'\033[0m'
fail=0

"$PY_BIN" - "$CSS" <<'PY'
import re
import sys

path = sys.argv[1]
text = open(path, encoding="utf-8").read()


def block(selector):
    """The body of a rule, found by its selector."""
    i = text.index(selector + " {")
    depth = 0
    for j in range(i, len(text)):
        if text[j] == "{":
            depth += 1
        elif text[j] == "}":
            depth -= 1
            if depth == 0:
                return text[i:j]
    raise SystemExit("unclosed block: " + selector)


light = block(":root")
dark = block(':root[data-theme="dark"]')

TOKEN = re.compile(r"^\s*(--[a-z0-9-]+):\s*([^;]+);", re.M)
COLOUR = re.compile(r"#[0-9a-fA-F]{3,8}\b|\brgba?\(|\bhsla?\(")

light_tokens = {m.group(1): m.group(2).strip() for m in TOKEN.finditer(light)}
dark_tokens = {m.group(1): m.group(2).strip() for m in TOKEN.finditer(dark)}

problems = []

# 1. Every colour token of the light theme has a counterpart in the dark one.
for name, value in light_tokens.items():
    if not COLOUR.search(value):
        continue  # not a colour: radii, speeds, easing curves
    if name not in dark_tokens:
        problems.append("token %s is in the light theme and missing from the dark one" % name)

# 2. Not one colour written out directly outside the token blocks.
rest = text.replace(light, "").replace(dark, "")
for n, line in enumerate(rest.splitlines(), 1):
    stripped = line.strip()
    if stripped.startswith("*") or stripped.startswith("/*") or stripped.startswith("//"):
        continue
    if COLOUR.search(line):
        problems.append("a colour written out outside the tokens: %s" % stripped)

if problems:
    for p in problems:
        print("  " + p)
    sys.exit(1)

print("  %d tokens in the light theme, %d of them colours with a counterpart" % (
    len(light_tokens),
    sum(1 for n, v in light_tokens.items() if COLOUR.search(v)),
))
PY
status=$?

if [[ $status -ne 0 ]]; then
	echo "${RED}--- the two themes: do NOT line up ---${OFF}" >&2
	fail=1
else
	echo "${GREEN}--- the two themes: line up ---${OFF}"
fi

exit $fail
