#!/usr/bin/env bash
# T361 — the addresses where the FFmpeg source can be had still answer.
#
# **Why at release time and not once, when they were written down.** Handing somebody a build
# with GPL parts in it obliges us to tell them where the source is. `THIRD-PARTY.md` does tell
# them, and both addresses point at immovable things — a commit and a build tag, not a branch
# that walks away. But "written down correctly" and "still answers" are different claims, and
# only the second one is any use to the person following the link. Repositories get renamed,
# made private, and taken down; none of that edits our file.
#
# Checking costs a second and is done for every release, because that is the moment somebody
# new is given a copy and the obligation starts over.
#
# **Not part of `check-all`**: it needs the network, and the everyday checks do not. A check
# that fails on a train teaches people to skip checks.
set -euo pipefail

APP="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RED=$'\033[31m'; GREEN=$'\033[32m'; OFF=$'\033[0m'

# Everything linked from the FFmpeg section: the source commit and the build scripts. Taken
# from the file rather than written here as well — two copies of an address is one address that
# will be updated and one that will not.
mapfile -t urls < <(
	sed -n '/^## FFmpeg/,/^## [^F]/p' "$APP/THIRD-PARTY.md" |
		grep -o 'https://[^)]*' |
		sort -u
)

if [ "${#urls[@]}" -eq 0 ]; then
	echo "${RED}--- the source addresses: NONE FOUND in THIRD-PARTY.md ---${OFF}" >&2
	echo "${RED}  The FFmpeg section links nowhere. That is the obligation missing, not passing.${OFF}" >&2
	exit 1
fi

fail=0
for url in "${urls[@]}"; do
	# Redirects are followed: GitHub answers a renamed repository with one, and the source is
	# genuinely there. What is not tolerated is 404, 403 and a host that does not answer.
	if curl -fsSIL --max-time 20 -o /dev/null "$url"; then
		echo "  answers: $url"
	else
		echo "${RED}  does not answer: $url${OFF}" >&2
		fail=1
	fi
done

if [ "$fail" -ne 0 ]; then
	echo "${RED}--- the source addresses: NOT all answering ---${OFF}" >&2
	echo "${RED}  Somebody handed this build has a right to that source. Fix the address in${OFF}" >&2
	echo "${RED}  THIRD-PARTY.md, or put the source somewhere that answers, before releasing.${OFF}" >&2
	exit 1
fi

echo "${GREEN}--- the source addresses: all answering (${#urls[@]}) ---${OFF}"
