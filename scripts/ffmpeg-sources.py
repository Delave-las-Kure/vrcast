# -*- coding: utf-8 -*-
"""T719 — the source of the bundled FFmpeg, carried by every release of ours.

**Why our own copy rather than a link** (owner's decision 2026-10-09). We hand out a built
FFmpeg under the GPL, so whoever gets it is owed the source it was built from. That used to be
two links into other people's repositories, and one of them — a BtbN build tag — was deleted
by its author a few weeks later, as BtbN does with daily builds. The obligation broke without
a single line of ours changing. A file attached to our own release lives as long as the
release does.

What goes in, both named in `scripts/ffmpeg.json`:

  - FFmpeg at the pinned commit — the program itself, built unmodified;
  - BtbN's build scripts at the pinned commit — how it was built, and the exact commit of
    every library inside it (x264 and the rest), `scripts.d/*.sh`.

**Packed from git, not downloaded as somebody's tarball.** `git fetch` of a commit hash checks
every object against that hash, so what lands in the archive is provably the pinned source,
whatever the address returned. A ready-made archive from a web address would be whatever the
address happened to serve that day, and its bytes are not even promised to stay the same.

To run it:
  python3 scripts/ffmpeg-sources.py check            — the pinned points still answer
  python3 scripts/ffmpeg-sources.py pack --out DIR   — make the two archives

Only the standard library and `git`: this runs on a release machine, at a poor moment to find
out a dependency has moved.
"""

import argparse
import json
import pathlib
import shutil
import subprocess
import sys
import tarfile
import tempfile
import urllib.error
import urllib.request

APP = pathlib.Path(__file__).resolve().parent.parent
MANIFEST = json.loads((APP / "scripts" / "ffmpeg.json").read_text(encoding="utf-8"))

# Below these sizes an archive is not the source, whatever it is called. FFmpeg's is tens of
# megabytes, BtbN's scripts a few hundred kilobytes.
MIN_BYTES = {"ffmpeg": 5_000_000, "build_scripts": 20_000}

# One file each archive must hold, to show it is what it says: a misplaced commit hash
# (somebody else's repository, a branch tip) would pack happily and be useless.
MUST_HOLD = {"ffmpeg": "configure", "build_scripts": "scripts.d/50-x264.sh"}


def say(line: str) -> None:
    print(line, flush=True)


def git(*args: str, cwd: pathlib.Path = None) -> str:
    done = subprocess.run(
        ["git", *args], cwd=cwd, capture_output=True, text=True, encoding="utf-8"
    )
    if done.returncode != 0:
        raise SystemExit("git %s -> %s" % (" ".join(args), done.stderr.strip()))
    return done.stdout.strip()


def answers(url: str) -> str:
    """Empty when the address answers (redirects followed), the reason otherwise."""
    request = urllib.request.Request(url, method="HEAD", headers={"User-Agent": "vrcast-release"})
    try:
        with urllib.request.urlopen(request, timeout=30) as answer:
            return "" if answer.status < 400 else "HTTP %d" % answer.status
    except urllib.error.HTTPError as e:
        return "HTTP %d" % e.code
    except (urllib.error.URLError, OSError) as e:
        return str(getattr(e, "reason", e))


def items():
    """(key, repository, commit, top directory) of what is packed."""
    s = MANIFEST["source"]
    return [
        ("ffmpeg", s["ffmpeg_repo"], s["ffmpeg_commit"], "ffmpeg-%s" % MANIFEST["version"]),
        (
            "build_scripts",
            s["build_scripts_repo"],
            s["build_scripts_commit"],
            "FFmpeg-Builds-%s" % MANIFEST["release_tag"],
        ),
    ]


def check() -> int:
    """Before a release: everything the release will fetch from outside still answers."""
    s = MANIFEST["source"]
    problems = []

    # The programs themselves: the installers cannot be built without them.
    for key, entry in MANIFEST["platforms"].items():
        url = "%s/releases/download/%s/%s" % (
            s["build_scripts_repo"],
            MANIFEST["release_tag"],
            entry["asset"],
        )
        why = answers(url)
        if why:
            problems.append("the %s build does not answer (%s): %s" % (key, why, url))
        else:
            say("  answers: %s" % url)

    # The tag still names the commit we pinned. A tag that moved means the programs above may
    # no longer be the ones the pinned scripts made.
    line = git("ls-remote", s["build_scripts_repo"], "refs/tags/%s" % MANIFEST["release_tag"])
    got = line.split()[0] if line else ""
    if got != s["build_scripts_commit"]:
        problems.append(
            "the tag %s names %s, and %s is pinned"
            % (MANIFEST["release_tag"], got or "nothing", s["build_scripts_commit"])
        )
    else:
        say("  the tag %s is still commit %s" % (MANIFEST["release_tag"], got[:10]))

    # The two commits the source is packed from.
    for key, repo, commit, _ in items():
        url = "%s/commit/%s" % (repo, commit)
        why = answers(url)
        if why:
            problems.append("the %s commit does not answer (%s): %s" % (key, why, url))
        else:
            say("  answers: %s" % url)

    if problems:
        for p in problems:
            print("  " + p, file=sys.stderr)
        return 1
    return 0


def pack(out: pathlib.Path) -> int:
    """Make the two archives a release carries, each verified before it is called done."""
    out.mkdir(parents=True, exist_ok=True)
    names = MANIFEST["source"]["release_files"]
    work = pathlib.Path(tempfile.mkdtemp(prefix="vrcast-ffmpeg-src-"))
    try:
        for key, repo, commit, top in items():
            repo_dir = work / key
            git("init", "-q", str(repo_dir))
            say("fetching %s %s…" % (repo, commit[:10]))
            git("fetch", "-q", "--depth", "1", repo, commit, cwd=repo_dir)
            got = git("rev-parse", "FETCH_HEAD^{commit}", cwd=repo_dir)
            if got != commit:
                raise SystemExit("asked for %s, git fetched %s" % (commit, got))

            target = out / names[key]
            git(
                "archive",
                "--format=tar.gz",
                "--prefix=%s/" % top,
                "-o",
                str(target),
                "FETCH_HEAD",
                cwd=repo_dir,
            )

            size = target.stat().st_size
            if size < MIN_BYTES[key]:
                raise SystemExit("%s is %d bytes — that is not the source" % (target.name, size))
            with tarfile.open(target, "r:gz") as tar:
                members = set(tar.getnames())
            want = "%s/%s" % (top, MUST_HOLD[key])
            if want not in members:
                raise SystemExit("%s holds no %s — not the source it is named for" % (target.name, want))
            say("  %s: %.1f MB, %d files, %s/" % (target.name, size / 1e6, len(members), top))
    finally:
        shutil.rmtree(work, ignore_errors=True)
    return 0


def main() -> int:
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="what", required=True)
    sub.add_parser("check")
    p = sub.add_parser("pack")
    p.add_argument("--out", required=True, type=pathlib.Path)
    args = ap.parse_args()
    return check() if args.what == "check" else pack(args.out)


if __name__ == "__main__":
    sys.exit(main())
