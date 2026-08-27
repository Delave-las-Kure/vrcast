#!/usr/bin/env bash
# T341, T344 — the application's version is written down exactly once.
#
# It used to be written three times: `src-tauri/Cargo.toml`, `src-tauri/tauri.conf.json` and
# `package.json`. They agreed by hand, which is to say by luck.
#
# **Why that is not cosmetic.** The running application reports `CARGO_PKG_VERSION`, and the
# "About" screen builds the source-code link out of it — `v{version}`, a git tag (T107). That
# link is a GPL obligation towards whoever was handed the build, and it has to lead to the
# source of **that** build. The installer, meanwhile, took its version from the Tauri config.
# Two numbers, one of them shown to the person, the other stamped on what they installed: let
# them drift and the obligation is broken while looking kept.
#
# So there is one number now, in `Cargo.toml`:
#
#   - the Tauri config has **no** `version` key at all — the schema says that when it is
#     removed, "the version number from `Cargo.toml` is used";
#   - `package.json` has none either. The package is private, npm does not require it, and
#     nothing in the repository reads it.
#
# This check exists because both of those are absences, and an absence is easy to undo by
# accident: a `npm version` run, a merge, a helpful editor. What it guards is that nobody has
# put a second copy back.
set -euo pipefail

APP="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RED=$'\033[31m'; GREEN=$'\033[32m'; OFF=$'\033[0m'
fail=0

say_no() { echo "${RED}  $1${OFF}" >&2; fail=1; }

# ---- the one place ----
version=$(sed -n 's/^version = "\([^"]*\)".*/\1/p' "$APP/src-tauri/Cargo.toml" | head -1)
if [ -z "$version" ]; then
	say_no "src-tauri/Cargo.toml has no version, and it is the one place that must have one"
	echo "${RED}--- the version: NOT in one place ---${OFF}" >&2
	exit 1
fi

# ---- and nowhere else ----
#
# Matched at the top level of the JSON, by indentation: a dependency's "version" field deeper
# in the file is somebody else's number and none of our business.
if grep -qE '^  "version"' "$APP/src-tauri/tauri.conf.json"; then
	say_no "src-tauri/tauri.conf.json has a version of its own again."
	say_no "  Remove it: with no key there, Tauri takes the number from Cargo.toml."
fi

if grep -qE '^  "version"' "$APP/package.json"; then
	say_no "package.json has a version of its own again."
	say_no "  Remove it: the package is private, and nothing here reads that field."
fi

# ---- and the tag, when standing on one ----
#
# Only when HEAD actually has a tag. On an ordinary commit there is nothing to compare against,
# and inventing a comparison would make the check fail for everybody every day.
tag=$(git -C "$APP" describe --exact-match --tags HEAD 2>/dev/null || true)
if [ -n "$tag" ]; then
	if [ "$tag" != "v$version" ]; then
		say_no "the tag is $tag and the version is $version."
		say_no "  The About screen sends people to v$version for the source of this build."
	fi
fi

if [ "$fail" -ne 0 ]; then
	echo "${RED}--- the version: NOT in one place ---${OFF}" >&2
	exit 1
fi

echo "  version $version, in one place${tag:+, tag $tag}"
echo "${GREEN}--- the version: in one place ---${OFF}"
