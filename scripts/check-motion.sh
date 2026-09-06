#!/usr/bin/env bash
# T516 — the movement switch must reach everything its label says it reaches.
#
# ⚠ **Found 2026-09-06, and the switch had been doing half its job since it was written.** Its
# own words are "transitions between sections and the mascot's movement". It reached the first
# half. The mascot's four transitions and its nod sat bare inside
# `@media (prefers-reduced-motion: no-preference)`, answering only to the system-wide setting —
# and `data-motion` was on `<main class="content">` while the mascot lives in the side panel,
# so no selector from there could have reached it in any case. A switch that does half of what
# it promises is worse than one that does nothing: the half that works is what persuades
# somebody the other half is their hardware.
#
# **The rule, and why it is exactly the right one.** Everything inside the `no-preference`
# block is there because it is movement — that is what the block means. So every rule in it
# must be gated by `[data-motion="on"]`: no exceptions, nothing to argue about. Motion outside
# that block is a different question and deliberately not this script's; the label promises two
# things, and quietly broadening a switch is its own fault.
#
# **A script and not a test**, for the reason `check-theme.sh` is one: Vite hands `?raw` on a
# stylesheet back as an empty string, so a test that read it this way would check nothing while
# looking thorough. Tried, measured, and abandoned.
set -euo pipefail

CSS="$(dirname "$0")/../src/app/styles.css"
APP="$(dirname "$0")/../src/app/App.tsx"

# **`python3`, then `python`.** Ubuntu ships only the first, Windows installers only the
# second, and a script naming one of them works on one machine of the two.
PY_BIN=$(command -v python3 || command -v python || true)
RED=$'\033[31m'; GREEN=$'\033[32m'; OFF=$'\033[0m'
if [ -z "$PY_BIN" ]; then
	echo "${RED}--- the movement switch: NOT CHECKED (no python on this machine) ---${OFF}" >&2
	exit 1
fi

"$PY_BIN" - "$CSS" "$APP" <<'PY'
import re
import sys

css = open(sys.argv[1], encoding="utf-8").read()
app = open(sys.argv[2], encoding="utf-8").read()

SWITCH = '[data-motion="on"]'
BLOCK = "@media (prefers-reduced-motion: no-preference)"
bad = []


def body(text, marker):
    """What is between the braces of the block that begins with `marker`."""
    at = text.index(marker)
    open_at = text.index("{", at)
    depth = 0
    for i in range(open_at, len(text)):
        if text[i] == "{":
            depth += 1
        elif text[i] == "}":
            depth -= 1
            if depth == 0:
                return text[open_at + 1:i]
    raise SystemExit("the %s block is never closed" % marker)


def selectors(block):
    """The selector of every rule at the top level of a block.

    Braces are counted rather than matched with an expression: the block holds nested rules,
    and a lazy `.*?}` would stop at the first inner brace and check a fraction of it while
    looking thorough.
    """
    text = re.sub(r"/\*.*?\*/", "", block, flags=re.S)
    out, head, depth = [], "", 0
    for c in text:
        if c == "{":
            depth += 1
            if depth == 1:
                out.append(head.strip())
                head = ""
                continue
        if c == "}":
            depth -= 1
            continue
        if depth == 0:
            head += c
    return [s for s in out if s]


if BLOCK not in css:
    raise SystemExit("the stylesheet has no %s block" % BLOCK)

rules = selectors(body(css, BLOCK))

# The check must find something, or everything below passes over an empty list — a failure
# with no symptom of its own, and the worst kind there is.
if len(rules) < 3:
    raise SystemExit(
        "only %d rules were found inside the reduce-motion block: the reader has stopped "
        "matching the shape they are written in" % len(rules)
    )

for selector in rules:
    parts = [p.strip() for p in selector.split(",") if p.strip()]
    if not all(SWITCH in p for p in parts):
        bad.append("moves without asking the setting: %s" % " ".join(selector.split()))

# The mascot half, which a selector alone cannot state. `[data-motion="on"] .mascot…` is true
# as text and false on screen if the attribute sits on an element the mascot is not inside. So
# the switch must be an ANCESTOR — a space after it — and the attribute must be on the layout,
# which holds the side panel as well as the content.
mascot = [s for s in rules if ".mascot" in s]
if not mascot:
    bad.append(
        "no rule in the reduce-motion block mentions the mascot: either it stopped moving, "
        "in which case this paragraph should go, or the reader has stopped finding it"
    )
for selector in mascot:
    for part in [p.strip() for p in selector.split(",") if p.strip()]:
        if not part.startswith(SWITCH + " "):
            bad.append(
                'the switch must be an ancestor of "%s", with a space after it: written '
                "without one it asks the mascot to carry the attribute, which nothing gives "
                "it" % part
            )

if 'className="layout" data-motion=' not in app:
    bad.append(
        "data-motion is not on the layout in App.tsx. On `.content` it cannot reach the "
        "mascot, which is in the side panel — that is the whole of T516"
    )

if bad:
    print("\n".join(bad))
    raise SystemExit(1)
print("the movement switch reaches every rule that moves (%d checked)" % len(rules))
PY

echo "${GREEN}--- the movement switch: every rule that moves obeys it ---${OFF}"
