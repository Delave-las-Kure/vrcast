#!/usr/bin/env bash
# The workflow files, checked before they are pushed rather than after.
#
# **Why this exists.** A workflow with a bad expression is not rejected step by step: the whole
# run fails with **no jobs at all** and nothing readable in the API — no annotation, no check
# run, no message. Twice in a row on 2026-08-27 that cost a push, a wait and a guess, and the
# second guess was wrong. `actionlint` named the fault in one line: `runner.workspace` is not
# available in the workflow-level `env` block, and has no such property in the first place.
#
# It runs in a container. Installing a Go tool on the machine to check a file would repeat the
# mistake this whole arrangement was rebuilt to avoid.
#
# Not run rather than failed when Docker is absent — the same shape as the other checks that
# need something the machine may not have. Said out loud, never passed off as a pass.
set -euo pipefail

APP="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RED=$'\033[31m'; GREEN=$'\033[32m'; YELLOW=$'\033[33m'; OFF=$'\033[0m'

# ⚠ **A missing Docker is a failure here, not a shrug** (T489, 2026-09-04). This used to
# print a yellow line and exit 0 — and `check-all.sh`, which reads only the exit code, then
# announced "passed: The workflow files" over a check that had not run. That is the exact
# shape this project keeps finding and fixing elsewhere; it had no business surviving in the
# runner of the checks themselves.
if ! docker info >/dev/null 2>&1; then
	echo "${RED}--- the workflows: NOT CHECKED, and that is a failure ---${OFF}" >&2
	echo "Docker is not running, so actionlint could not be reached. Start it: a workflow" >&2
	echo "with a bad expression fails the whole run with no jobs and nothing readable." >&2
	exit 1
fi

# The image is pinned: an unpinned linter starts objecting to what it did not object to
# yesterday, and a check that changes its mind on its own is a check people stop believing.
IMAGE="rhysd/actionlint:1.7.7"

# The labels of the self-hosted runners live in .github/actionlint.yaml, which the tool reads
# from the repository it is pointed at.
# **The files are named rather than left to actionlint's own search.** With no argument it
# lints whatever it finds beneath the working directory and says nothing at all when it finds
# nothing — so a bind mount that silently went wrong reads exactly like a sound workflow.
FILES=()
while IFS= read -r f; do FILES+=(".github/workflows/$f"); done < <(
	cd "$APP/.github/workflows" && ls -1 *.yml *.yaml 2>/dev/null
)
if [ ${#FILES[@]} -eq 0 ]; then
	echo "${RED}--- the workflows: none found under $APP/.github/workflows ---${OFF}" >&2
	exit 1
fi

if MSYS_NO_PATHCONV=1 docker run --rm -v "/${APP}:/repo" -w /repo "$IMAGE" -color "${FILES[@]}"; then
	echo "${GREEN}--- the workflows: sound ---${OFF}"
else
	echo "${RED}--- the workflows: NOT sound ---${OFF}" >&2
	exit 1
fi
