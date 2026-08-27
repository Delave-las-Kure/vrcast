#!/usr/bin/env bash
# T362 — the release file assembler, checked on made-up bundles.
#
# **Why this is worth a test of its own.** The script runs once per release, on a machine
# nobody is watching, at the one moment where a mistake is expensive: a `latest.json` short of
# a platform does not fail anything — it publishes, and everybody on that platform silently
# stops being offered updates. There is no second chance to notice, because nothing complains.
#
# So the cases here are the failures, not the success: a platform with no file, a file with no
# signature, an empty signature, two candidates for one platform, and the Linux case where
# both shapes are lying about and only one is signed. The success case is checked
# for the two things that actually have to be right — the key for the package copy, and each
# signature landing on its own artefact.
set -euo pipefail

APP="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RED=$'\033[31m'; GREEN=$'\033[32m'; OFF=$'\033[0m'
PY=$(command -v python3 || command -v python)

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
fail=0

say_no() { echo "${RED}  $1${OFF}" >&2; fail=1; }

full() {
	local dir="$1"
	rm -rf "$dir"; mkdir -p "$dir"
	printf 'exe' >"$dir/VRCast Studio_1.2.3_x64-setup.exe"
	printf 'sig-win' >"$dir/VRCast Studio_1.2.3_x64-setup.exe.sig"
	printf 'img' >"$dir/vrcast-studio_1.2.3_amd64.AppImage"
	printf 'sig-appimage' >"$dir/vrcast-studio_1.2.3_amd64.AppImage.sig"
	printf 'deb' >"$dir/vrcast-studio_1.2.3_amd64.deb"
	printf 'sig-deb' >"$dir/vrcast-studio_1.2.3_amd64.deb.sig"
}

run() {
	"$PY" "$APP/scripts/latest-json.py" \
		--version 1.2.3 --tag v1.2.3 --repo owner/name \
		--pub-date 2026-08-28T00:00:00Z \
		--bundles "$1" --out "$work/latest.json" "${@:2}"
}

# --- all three there ------------------------------------------------------------------------
full "$work/ok"
if run "$work/ok" >/dev/null; then
	got=$("$PY" -c "import json,sys; d=json.load(open(sys.argv[1], encoding='utf-8')); print(' '.join(sorted(d['platforms'])))" "$work/latest.json")
	want="linux-x86_64 linux-x86_64-deb windows-x86_64"
	[ "$got" = "$want" ] || say_no "the platforms are [$got], and they should be [$want]"

	# Each signature has to belong to its own file. Swapped, the update downloads and is
	# refused — after the download, on the person's machine, for no visible reason.
	deb_sig=$("$PY" -c "import json,sys; print(json.load(open(sys.argv[1], encoding='utf-8'))['platforms']['linux-x86_64-deb']['signature'])" "$work/latest.json")
	[ "$deb_sig" = "sig-deb" ] || say_no "the .deb carries the signature [$deb_sig]"

	deb_url=$("$PY" -c "import json,sys; print(json.load(open(sys.argv[1], encoding='utf-8'))['platforms']['linux-x86_64-deb']['url'])" "$work/latest.json")
	case "$deb_url" in
		*"/releases/download/v1.2.3/"*".deb") : ;;
		*) say_no "the .deb address is $deb_url" ;;
	esac
else
	say_no "a complete set of bundles was refused"
fi

# --- a platform missing ---------------------------------------------------------------------
full "$work/no-deb"; rm "$work/no-deb/vrcast-studio_1.2.3_amd64.deb" "$work/no-deb/vrcast-studio_1.2.3_amd64.deb.sig"
if run "$work/no-deb" >/dev/null 2>&1; then
	say_no "a set with no .deb was accepted — that release would drop every package copy"
fi

# --- a file with no signature ---------------------------------------------------------------
full "$work/unsigned"; rm "$work/unsigned/vrcast-studio_1.2.3_amd64.AppImage.sig"
if run "$work/unsigned" >/dev/null 2>&1; then
	say_no "an unsigned AppImage was accepted"
fi

# --- an empty signature ---------------------------------------------------------------------
full "$work/empty"; : >"$work/empty/vrcast-studio_1.2.3_amd64.deb.sig"
if run "$work/empty" >/dev/null 2>&1; then
	say_no "an empty signature was accepted"
fi

# --- both Linux shapes present, one of them signed ------------------------------------------
#
# The bundler makes either the v1-compatible archive or the AppImage signed directly, and a
# build directory kept between runs can hold yesterday's other shape. The signed one is the
# artefact; stopping at the unsigned one would fail a release that is perfectly complete.
full "$work/both"; printf 'archive' >"$work/both/vrcast-studio_1.2.3_amd64.AppImage.tar.gz"
if run "$work/both" >/dev/null; then
	picked=$("$PY" -c "import json,sys; print(json.load(open(sys.argv[1], encoding='utf-8'))['platforms']['linux-x86_64']['url'].rsplit('/',1)[1])" "$work/latest.json")
	case "$picked" in
		*.AppImage) : ;;
		*) say_no "an unsigned $picked was taken over the signed AppImage" ;;
	esac
else
	say_no "an unsigned leftover archive beside a signed AppImage was treated as a failure"
fi

# --- two candidates for one platform --------------------------------------------------------
full "$work/twice"; printf 'deb' >"$work/twice/vrcast-studio_1.2.4_amd64.deb"
printf 'sig' >"$work/twice/vrcast-studio_1.2.4_amd64.deb.sig"
if run "$work/twice" >/dev/null 2>&1; then
	say_no "two .deb files were accepted — one of them would go out unnoticed"
fi

if [ "$fail" -ne 0 ]; then
	echo "${RED}--- the release file: NOT sound ---${OFF}" >&2
	exit 1
fi
echo "${GREEN}--- the release file: assembled and refused in the right places ---${OFF}"
