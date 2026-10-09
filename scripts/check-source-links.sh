#!/usr/bin/env bash
# T361, T719 — whoever is handed a build can actually get the FFmpeg source.
#
#   bash scripts/check-source-links.sh               before a release: what it will fetch answers
#   bash scripts/check-source-links.sh --published   after it: the attached source downloads
#
# **Why at release time and not once, when they were written down.** Handing somebody a build
# with GPL parts in it obliges us to tell them where the source is. "Written down correctly"
# and "still answers" are different claims, and only the second one is any use to the person
# following the link. It was learned the hard way (T719, 2026-10-09): the BtbN build tag the
# links named was deleted by its author a few weeks after it was pinned, and the release
# stopped here.
#
# Since then the source travels **with each release** (owner's decision 2026-10-09):
# `scripts/ffmpeg-sources.py` packs FFmpeg at the pinned commit and BtbN's build scripts at
# theirs, and the publishing job attaches both. THIRD-PARTY.md links to those two files of
# this very release. So the check has two halves, at the two moments they can be answered:
#
#   - **before** — THIRD-PARTY.md names this repository and this version (or the links lead
#     to some other release), and everything outside that the release will need still
#     answers: both BtbN builds (the installers are built from them), the tag still on the
#     pinned commit, both pinned commits. Failing here costs a minute; failing in the
#     publishing job costs the whole build before it;
#   - **after** (`--published`) — the files the links name download for anybody, without any
#     permission, like `latest.json`. A link to a release asset that is not there is the old
#     failure in a new place.
#
# **Not part of `check-all`**: it needs the network, and the everyday checks do not. A check
# that fails on a train teaches people to skip checks.
set -euo pipefail

APP="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RED=$'\033[31m'; GREEN=$'\033[32m'; OFF=$'\033[0m'
PY=$(command -v python3 || command -v python || true)

PUBLISHED=0
case "${1:-}" in
	--published) PUBLISHED=1 ;;
	"") ;;
	*) echo "usage: $0 [--published]" >&2; exit 2 ;;
esac

version=$(sed -n 's/^version = "\([^"]*\)".*/\1/p' "$APP/src-tauri/Cargo.toml" | head -1)
# The repository the release is published to: the one this run belongs to on GitHub, the one
# the updates are read from anywhere else. Not written here a third time.
repo="${GITHUB_REPOSITORY:-$(sed -n 's|.*"https://github.com/\([^/]*/[^/]*\)/releases/.*|\1|p' "$APP/src-tauri/tauri.conf.json" | head -1)}"
base="https://github.com/$repo/releases/download/v$version/"

# The links of the FFmpeg section — markdown link targets only. Taken from the file rather than
# written here as well: two copies of an address is one address that will be updated and one
# that will not.
mapfile -t urls < <(
	sed -n '/^## FFmpeg/,/^## [^F]/p' "$APP/THIRD-PARTY.md" |
		grep -o '](https://[^)]*)' | sed 's/^](//; s/)$//' |
		sort -u
)

if [ "${#urls[@]}" -eq 0 ]; then
	echo "${RED}--- the source addresses: NONE FOUND in THIRD-PARTY.md ---${OFF}" >&2
	echo "${RED}  The FFmpeg section links nowhere. That is the obligation missing, not passing.${OFF}" >&2
	exit 1
fi

fail=0

if [ "$PUBLISHED" -eq 1 ]; then
	work=$(mktemp -d)
	trap 'rm -rf "$work"' EXIT
	for url in "${urls[@]}"; do
		# Downloaded whole, without a token, redirects followed: the question is about a
		# stranger with a browser, and a HEAD that answers says less than a file that arrives.
		if size=$(curl -fsSL --max-time 300 -o "$work/file" -w '%{size_download}' "$url") &&
			[ "${size%.*}" -gt 0 ]; then
			echo "  downloads for anybody ($((${size%.*} / 1024)) KB): $url"
		else
			echo "${RED}  does not download: $url${OFF}" >&2
			fail=1
		fi
		rm -f "$work/file"
	done
	if [ "$fail" -ne 0 ]; then
		echo "${RED}--- the attached source: NOT all downloading ---${OFF}" >&2
		echo "${RED}  The release is out and THIRD-PARTY.md promises these files. Attach them to it${OFF}" >&2
		echo "${RED}  (python3 scripts/ffmpeg-sources.py pack --out DIR), or the obligation is broken.${OFF}" >&2
		exit 1
	fi
	echo "${GREEN}--- the attached source: downloads for anybody (${#urls[@]}) ---${OFF}"
	exit 0
fi

# ---- before the release: the links name this release ----
for url in "${urls[@]}"; do
	case "$url" in
		"$base"*) echo "  names this release: $url" ;;
		*)
			echo "${RED}  names another place: $url${OFF}" >&2
			echo "${RED}    expected under $base — run \`npm run third-party\` and commit the file.${OFF}" >&2
			fail=1
			;;
	esac
done

# ---- and what the release will fetch from outside still answers ----
if [ -z "$PY" ]; then
	echo "${RED}  no python3 here, so the pinned FFmpeg addresses could not be checked${OFF}" >&2
	fail=1
elif ! "$PY" "$APP/scripts/ffmpeg-sources.py" check; then
	fail=1
fi

if [ "$fail" -ne 0 ]; then
	echo "${RED}--- the source addresses: NOT all in order ---${OFF}" >&2
	echo "${RED}  Somebody handed this build has a right to its source. If a pinned BtbN build is${OFF}" >&2
	echo "${RED}  gone, re-pin scripts/ffmpeg.json to the last build of a finished month.${OFF}" >&2
	exit 1
fi

echo "${GREEN}--- the source addresses: in order (${#urls[@]} links, pinned points answering) ---${OFF}"
