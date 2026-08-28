#!/usr/bin/env bash
# The rule that decides whether a build goes red over a leaked Docker network.
#
# **Why it is checked at all.** The guard in build.yml used to fail a job for any network
# matching the prefix, wherever it came from. The runner is our own, so a network leaked once
# stays on it for good — the name carries the process id that made it, and no later run ever
# meets it again — and every build after that one was red for somebody else's leak. A build
# that is always red teaches people not to read red, which is the same argument already
# written into that guard about a flickering check.
#
# So the guard now judges only what the run itself left, and prints the older ones separately.
# That rule is the difference between a red build and a green one, and it is four lines of
# shell in a YAML string, where nothing else would ever look at it.
#
# The three-line function below is a copy of those four lines. A copy is not ideal; the
# alternative is that the rule is checked by nothing at all.
set -euo pipefail

GREEN=$'\033[32m'; RED=$'\033[31m'; OFF=$'\033[0m'

# The verdict, from what was there before and what is there now.
verdict() {
	local before="$1" now="$2" tmp fresh
	tmp="$(mktemp -d)"
	printf '%s' "$before" >"$tmp/before.txt"
	fresh="$(printf '%s
' "$now" | grep -vxF -f "$tmp/before.txt" || true)"
	rm -rf "$tmp"
	if [ -n "$fresh" ]; then echo "fresh"
	elif [ -n "$now" ]; then echo "old-only"
	else echo "clean"; fi
}

failed=0
check() {
	local want="$1" label="$2" before="$3" now="$4" got
	got="$(verdict "$before" "$now")"
	if [ "$got" != "$want" ]; then
		echo "${RED}FAIL${OFF} [$label]: wanted $want, got $got" >&2
		failed=1
	fi
}

check clean "a clean machine" "" ""

# What 2026-08-28 actually looked like: three networks from three earlier runs, and a run that
# added none of them. Not this run's verdict.
check old-only "only what was already here" \
	$'vrcast-deploy-target-342-0-net\nvrcast-deploy-target-931-0-net\n' \
	$'vrcast-deploy-target-342-0-net\nvrcast-deploy-target-931-0-net'

# And the case the guard exists for, which must still bite with old ones beside it.
check fresh "a new leak beside an old one" \
	$'vrcast-deploy-target-342-0-net\n' \
	$'vrcast-deploy-target-342-0-net\nvrcast-deploy-target-777-0-net'

check fresh "a new leak on a clean machine" "" $'vrcast-test-net-5'

# The way this rule could quietly stop biting: no record of what was there beforehand — the
# step did not run, or the file was lost — must mean everything counts as this run's, never
# nothing.
check fresh "no record of what was here" "" $'vrcast-deploy-target-1-0-net'

if [ "$failed" -eq 0 ]; then
	echo "${GREEN}--- the leak guard's rule: sound ---${OFF}"
else
	echo "${RED}--- the leak guard's rule: NOT sound ---${OFF}" >&2
	exit 1
fi
