# -*- coding: utf-8 -*-
"""T362 — assemble the `latest.json` that installed copies read to find a new version.

**Why a script and not a line in the workflow.** The file has to name one artefact per way the
application can be installed, and each name has to be paired with the signature made for that
exact file. Getting it wrong does not fail the release: it publishes a file that looks complete
and quietly serves the wrong thing — a `.deb` copy handed an AppImage, or a platform silently
missing so that everybody on it stops being offered updates and nobody hears about it.

So this refuses rather than improvises. Every platform it is asked for must have a file **and**
its signature; anything else is an error with the reason named.

**Why `linux-x86_64-deb` exists at all.** The updater asks for `{os}-{arch}-{installer}` first
and falls back to `{os}-{arch}`. With only the fallback present, a copy installed from the
package downloads the AppImage and tries to install it as one, which it is not. Read out of
`tauri-plugin-updater` 2.10.1 on 2026-08-28, not assumed.
"""

import argparse
import json
import pathlib
import sys
import urllib.parse

# What each platform key is allowed to be served by, best first. The order matters for Linux:
# when the bundler makes the v1-compatible archive, that is what the running copy expects, and
# the plain AppImage is the newer shape. Either is accepted; both together take the archive.
WANTED = {
    "windows-x86_64": ["*-setup.exe", "*.msi"],
    "linux-x86_64": ["*.AppImage.tar.gz", "*.AppImage"],
    "linux-x86_64-deb": ["*.deb"],
}


def pick(bundles: pathlib.Path, patterns: list):
    """The first artefact matching a pattern **that has a signature beside it**.

    Both halves matter. The Linux patterns name two shapes because the bundler makes one or the
    other depending on how updater artefacts are configured: the v1-compatible archive, or the
    AppImage signed directly. Whichever it made is the one with a `.sig` next to it — so a
    candidate without one is not the artefact, it is the other shape lying around, and stopping
    at it would fail a release that is perfectly complete.

    Returns `(artefact, signature)`, or `(None, reason)` when nothing usable is there.
    """
    seen = []
    for pattern in patterns:
        # Sorted so that two candidates never depend on the order the filesystem happens to
        # hand them back: a release that differs between machines is a release nobody can check.
        found = sorted(f for f in bundles.rglob(pattern) if not f.name.endswith(".sig"))
        if len(found) > 1:
            raise SystemExit(
                "more than one file matches %s: %s%sPick a single artefact per platform; two of "
                "them means the build made something it was not asked for."
                % (pattern, ", ".join(f.name for f in found), "\n")
            )
        for artefact in found:
            seen.append(artefact.name)
            signature = artefact.with_name(artefact.name + ".sig")
            if signature.is_file() and signature.read_text(encoding="utf-8").strip():
                return artefact, signature
    if seen:
        return None, "found %s, and not one of them carries a signature" % ", ".join(seen)
    return None, "looked for %s" % ", ".join(patterns)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--version", required=True, help="without the leading v")
    ap.add_argument("--tag", required=True)
    ap.add_argument("--repo", required=True, help="owner/name")
    ap.add_argument("--bundles", required=True, type=pathlib.Path)
    ap.add_argument("--notes", default="", help="what is in this version, as plain text")
    ap.add_argument("--pub-date", default=None, help="RFC3339; taken from the caller so that "
                                                     "two runs of this script agree")
    ap.add_argument("--out", required=True, type=pathlib.Path)
    args = ap.parse_args()

    if not args.bundles.is_dir():
        raise SystemExit("no such directory of bundles: %s" % args.bundles)

    platforms = {}
    missing = []
    for key, patterns in WANTED.items():
        artefact, signature = pick(args.bundles, patterns)
        if artefact is None:
            missing.append("%s: %s" % (key, signature))
            continue
        platforms[key] = {
            "signature": signature.read_text(encoding="utf-8").strip(),
            "url": "https://github.com/%s/releases/download/%s/%s"
            % (args.repo, urllib.parse.quote(args.tag), urllib.parse.quote(artefact.name)),
        }

    if missing:
        print("latest.json was not written. Missing:", file=sys.stderr)
        for line in missing:
            print("  " + line, file=sys.stderr)
        print(
            "\nA release short of a platform is not a smaller release: everybody on that "
            "platform stops being offered updates, and nothing says so.",
            file=sys.stderr,
        )
        return 1

    if args.pub_date:
        pub_date = args.pub_date
    else:
        import datetime

        pub_date = datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")

    args.out.write_text(
        json.dumps(
            {
                "version": args.version,
                "notes": args.notes,
                "pub_date": pub_date,
                "platforms": platforms,
            },
            ensure_ascii=False,
            indent=2,
        )
        + "\n",
        encoding="utf-8",
    )
    print("latest.json: %d platforms" % len(platforms))
    for key, value in platforms.items():
        print("  %-20s %s" % (key, value["url"].rsplit("/", 1)[1]))
    return 0


if __name__ == "__main__":
    sys.exit(main())
