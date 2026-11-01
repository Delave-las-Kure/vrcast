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
import hashlib
import json
import mimetypes
import os
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
        with urllib.request.urlopen(request, timeout=300) as answer:
            return json.load(answer)
    except urllib.error.HTTPError as e:
        detail = e.read().decode("utf-8", "replace")[:500]
        raise SystemExit("%s %s -> %s%s%s" % (method, url.split("?")[0], e.code, "\n", detail))


def digest(stream):
    result = hashlib.sha256()
    while chunk := stream.read(1024 * 1024):
        result.update(chunk)
    return result.hexdigest()


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


def download_sha256(token, repo, asset):
    """Download a draft asset with the job token; never forward it to a signed CDN URL."""
    url = f"{API}/repos/{repo}/releases/assets/{asset['id']}"
    request = urllib.request.Request(url, headers={
        "Authorization": f"Bearer {token}", "Accept": "application/octet-stream",
        "X-GitHub-Api-Version": "2022-11-28",
    })
    try:
        try:
            answer = urllib.request.build_opener(NoRedirect).open(request, timeout=300)
        except urllib.error.HTTPError as error:
            if error.code not in (301, 302, 303, 307, 308):
                raise
            location = error.headers.get("Location", "")
            parsed = urllib.parse.urlsplit(location)
            host = parsed.hostname or ""
            if parsed.scheme != "https" or not (host == "github.com" or host.endswith(".githubusercontent.com")):
                raise ValueError("Draft asset redirected outside GitHub") from None
            # No Authorization header beyond api.github.com.
            answer = urllib.request.urlopen(location, timeout=300)
        with answer:
            return digest(answer)
    except urllib.error.URLError:
        # Signed query strings must not enter logs, even on a failed download.
        raise ValueError(f"Could not download draft asset {asset['name']}") from None


def validate_latest(manifest, repo, tag, names):
    if tag != "v" + str(manifest.get("version", "")):
        raise ValueError("latest.json names a different version")
    platforms = manifest.get("platforms", {})
    if set(platforms) != {"windows-x86_64", "linux-x86_64", "linux-x86_64-deb"}:
        raise ValueError("latest.json does not contain all three supported installation types")
    addresses = {f"https://github.com/{repo}/releases/download/{urllib.parse.quote(tag)}/{urllib.parse.quote(name)}" for name in names}
    for platform, entry in platforms.items():
        if entry.get("url") not in addresses or not entry.get("signature", "").strip():
            raise ValueError(f"latest.json has no signed uploaded asset for {platform}")
        suffixes = {"windows-x86_64": ("-setup.exe", ".exe", ".msi"), "linux-x86_64": (".AppImage", ".AppImage.tar.gz"), "linux-x86_64-deb": (".deb",)}
        if not urllib.parse.urlsplit(entry["url"]).path.endswith(suffixes[platform]):
            raise ValueError(f"latest.json points {platform} at another installation type")


def verify_draft(token, repo, tag, release_id, uploads):
    """Read back the exact draft, download EVERY file and compare its bytes before PATCH."""
    release = call(token, "GET", f"{API}/repos/{repo}/releases/{release_id}")
    if not release.get("draft") or release.get("tag_name") != tag:
        raise ValueError("Expected this tag's unpublished draft")
    assets = release.get("assets", [])
    expected = {asset["name"]: (path, asset["id"]) for path, asset in uploads}
    if len(expected) != len(uploads) or len(assets) != len(uploads) or {a["name"] for a in assets} != set(expected):
        raise ValueError("The draft's files are missing, duplicated or unexpected")
    source_files = json.loads(pathlib.Path(__file__).with_name("ffmpeg.json").read_text(encoding="utf-8"))["source"]["release_files"]
    required = {"latest.json", "LICENSE", "THIRD-PARTY.md", *source_files.values()}
    if not required.issubset(expected):
        raise ValueError("The draft is missing update metadata, notices or the promised FFmpeg source")
    manifest = json.loads(expected["latest.json"][0].read_text(encoding="utf-8"))
    validate_latest(manifest, repo, tag, set(expected))
    for asset in assets:
        path, asset_id = expected[asset["name"]]
        if asset["id"] != asset_id or asset.get("state") != "uploaded" or asset.get("size") != path.stat().st_size:
            raise ValueError(f"Draft asset is incomplete: {asset['name']}")
        with path.open("rb") as local:
            expected_hash = digest(local)
        if download_sha256(token, repo, asset) != expected_hash:
            raise ValueError(f"Draft asset differs from the file built: {asset['name']}")
        print(f"  downloaded and verified while draft: {asset['name']}")


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--repo", required=True, help="owner/name")
    ap.add_argument("--tag", required=True)
    ap.add_argument("--name", default=None, help="defaults to the tag")
    ap.add_argument("--notes", default="")
    ap.add_argument("--token", default=os.environ.get("TOKEN") or os.environ.get("GITHUB_TOKEN"), help="defaults to TOKEN/GITHUB_TOKEN environment")
    ap.add_argument("files", nargs="+", type=pathlib.Path)
    args = ap.parse_args()
    if not args.token:
        ap.error("a job token is required (TOKEN or GITHUB_TOKEN)")

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
            "prerelease": False,
        },
    )
    upload = release["upload_url"].split("{", 1)[0]
    print("draft: %s" % release["html_url"])

    uploads = []
    for path in args.files:
        kind = mimetypes.guess_type(path.name)[0] or "application/octet-stream"
        asset = call(
            args.token,
            "POST",
            "%s?name=%s" % (upload, urllib.parse.quote(path.name)),
            content_type=kind,
            raw=path.read_bytes(),
        )
        uploads.append((path, asset))
        print("  attached: %s" % path.name)

    # /releases/latest/download is PUBLIC ONLY. Draft downloads use the authenticated asset
    # API above, checking the actual uploaded names (GitHub normalises spaces to dots).
    verify_draft(args.token, args.repo, args.tag, release["id"], uploads)
    call(
        args.token,
        "PATCH",
        "%s/repos/%s/releases/%d" % (API, args.repo, release["id"]),
        {"draft": False, "prerelease": False, "make_latest": "true"},
    )
    published = call(args.token, "GET", "%s/repos/%s/releases/%d" % (API, args.repo, release["id"]))
    if published.get("draft") is not False or published.get("tag_name") != args.tag:
        raise ValueError("GitHub did not confirm publication of the verified draft")
    print("published: %s" % release["html_url"])
    return 0


if __name__ == "__main__":
    sys.exit(main())
