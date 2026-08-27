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

if ! docker info >/dev/null 2>&1; then
	echo "${YELLOW}--- the workflows: NOT CHECKED (Docker is not running) ---${OFF}"
	exit 0
fi

# The image is pinned: an unpinned linter starts objecting to what it did not object to
# yesterday, and a check that changes its mind on its own is a check people stop believing.
IMAGE="rhysd/actionlint:1.7.7"

# The labels of the self-hosted runners live in .github/actionlint.yaml, which the tool reads
# from the repository it is pointed at.
if MSYS_NO_PATHCONV=1 docker run --rm -v "/${APP}:/repo" -w /repo "$IMAGE" -color; then
	echo "${GREEN}--- the workflows: sound ---${OFF}"
else
	echo "${RED}--- the workflows: NOT sound ---${OFF}" >&2
	exit 1
fi
