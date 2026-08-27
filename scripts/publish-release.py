# -*- coding: utf-8 -*-
"""T362 — publish the release: everything a person gets, beside everything they are owed.

**Draft first, and only then visible.** The release is created as a draft, the files are
attached, and it is opened to the world last. Any failure along the way leaves a draft nobody
was offered, rather than a half-published release with three files out of six — and a
`latest.json` already visible to installed copies, pointing at artefacts that are not there.

Only the standard library: this runs on a release machine, and a release is a poor moment to
find out that a dependency has moved.
"""

import argparse
import json
import mimetypes
import pathlib
import sys
import urllib.error
import urllib.parse
import urllib.request

API = "https://api.github.com"


def call(token: str, method: str, url: str, body=None, content_type=None, raw: bytes = None):
    headers = {
        "Authorization": "Bearer %s" % token,
        "Accept": "application/vnd.github+json",
        "X-GitHub-Api-Version": "2022-11-28",
    }
    if raw is not None:
        data = raw
        headers["Content-Type"] = content_type or "application/octet-stream"
    elif body is not None:
        data = json.dumps(body).encode("utf-8")
        headers["Content-Type"] = "application/json"
    else:
        data = None

    request = urllib.request.Request(url, data=data, headers=headers, method=method)
    try:
        with urllib.request.urlopen(request) as answer:
            return json.load(answer)
    except urllib.error.HTTPError as e:
        detail = e.read().decode("utf-8", "replace")[:500]
        raise SystemExit("%s %s -> %s%s%s" % (method, url.split("?")[0], e.code, "\n", detail))


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--repo", required=True, help="owner/name")
    ap.add_argument("--tag", required=True)
    ap.add_argument("--name", default=None, help="defaults to the tag")
    ap.add_argument("--notes", default="")
    ap.add_argument("--token", required=True)
    ap.add_argument("files", nargs="+", type=pathlib.Path)
    args = ap.parse_args()

    missing = [str(f) for f in args.files if not f.is_file()]
    if missing:
        raise SystemExit("these were to be published and are not there: %s" % ", ".join(missing))

    # A second release on the same tag would be two answers to "which build is v1.2.3".
    existing = call(
        args.token,
        "GET",
        "%s/repos/%s/releases" % (API, args.repo),
    )
    for release in existing:
        if release.get("tag_name") == args.tag:
            raise SystemExit(
                "there is already a release for %s (%s). Delete it first, deliberately, or "
                "release a new version." % (args.tag, release.get("html_url"))
            )

    release = call(
        args.token,
        "POST",
        "%s/repos/%s/releases" % (API, args.repo),
        {
            "tag_name": args.tag,
            "name": args.name or args.tag,
            "body": args.notes,
            "draft": True,
        },
    )
    upload = release["upload_url"].split("{", 1)[0]
    print("draft: %s" % release["html_url"])

    for path in args.files:
        kind = mimetypes.guess_type(path.name)[0] or "application/octet-stream"
        call(
            args.token,
            "POST",
            "%s?name=%s" % (upload, urllib.parse.quote(path.name)),
            content_type=kind,
            raw=path.read_bytes(),
        )
        print("  attached: %s" % path.name)

    call(
        args.token,
        "PATCH",
        "%s/repos/%s/releases/%d" % (API, args.repo, release["id"]),
        {"draft": False},
    )
    print("published: %s" % release["html_url"])
    return 0


if __name__ == "__main__":
    sys.exit(main())
