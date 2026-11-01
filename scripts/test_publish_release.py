"""Release publication must not make an incomplete or corrupt draft visible."""
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("publish_release", Path(__file__).with_name("publish-release.py"))
assert spec is not None and spec.loader is not None
release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release)


class DraftVerificationTests(unittest.TestCase):
    @unittest.skipUnless(os.environ.get("VRCAST_TEST_PUBLISHED_RELEASE") == "1", "read-only GitHub API smoke test is opt-in")
    def test_real_authenticated_asset_download_matches_the_public_asset(self):
        token = os.environ["TOKEN"]
        package = json.loads(Path(__file__).resolve().parent.parent.joinpath("package.json").read_text(encoding="utf-8"))
        repo = package["repository"]["url"].removeprefix("https://github.com/")
        current = release.call(token, "GET", f"{release.API}/repos/{repo}/releases/latest")
        asset = next(asset for asset in current["assets"] if asset["name"] == "latest.json")
        actual = release.download_sha256(token, repo, asset)
        with release.urllib.request.urlopen(asset["browser_download_url"], timeout=60) as public:
            self.assertEqual(actual, release.digest(public))

    def test_latest_requires_all_platforms_and_exact_uploaded_addresses(self):
        names = {"windows.exe", "linux.AppImage", "linux.deb"}
        manifest = {"version": "1.2.3", "platforms": {
            key: {"url": f"https://github.com/owner/repo/releases/download/v1.2.3/{name}", "signature": "signed"}
            for key, name in zip(["windows-x86_64", "linux-x86_64", "linux-x86_64-deb"], ["windows.exe", "linux.AppImage", "linux.deb"])
        }}
        release.validate_latest(manifest, "owner/repo", "v1.2.3", names)
        for broken in [
            {**manifest, "version": "1.2.2"},
            {**manifest, "platforms": {}},
            {**manifest, "platforms": {**manifest["platforms"], "windows-x86_64": {"url": "https://github.com/owner/repo/releases/latest/download/windows.exe", "signature": "signed"}}},
            {**manifest, "platforms": {**manifest["platforms"], "windows-x86_64": {"url": "https://github.com/owner/repo/releases/download/v1.2.3/missing.exe", "signature": "signed"}}},
        ]:
            with self.assertRaises(ValueError):
                release.validate_latest(broken, "owner/repo", "v1.2.3", names)

    def test_every_uploaded_file_is_downloaded_and_hashed_before_publishing(self):
        events = []
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            binaries = ["VRCast.Studio-setup.exe", "linux.AppImage", "linux.deb"]
            source_names = json.loads(Path(__file__).with_name("ffmpeg.json").read_text())["source"]["release_files"].values()
            names = binaries + ["latest.json", "LICENSE", "THIRD-PARTY.md"] + list(source_names)
            files = []
            for name in names:
                path = root / name
                path.write_bytes(b"contents of " + name.encode())
                files.append(path)
            manifest = {"version": "1.2.3", "platforms": {
                key: {"url": f"https://github.com/owner/repo/releases/download/v1.2.3/{name}", "signature": "signed"}
                for key, name in zip(["windows-x86_64", "linux-x86_64", "linux-x86_64-deb"], binaries)
            }}
            (root / "latest.json").write_text(json.dumps(manifest))
            assets = [{"id": i, "name": path.name, "size": path.stat().st_size, "state": "uploaded"} for i, path in enumerate(files)]
            draft = {"id": 10, "draft": True, "tag_name": "v1.2.3", "assets": assets}

            def download(token, repo, asset):
                events.append(asset["name"])
                return hashlib.sha256((root / asset["name"]).read_bytes()).hexdigest()

            uploads = list(zip(files, assets))
            with patch.object(release, "call", return_value=draft), patch.object(release, "download_sha256", side_effect=download):
                release.verify_draft("unit-test-token", "owner/repo", "v1.2.3", 10, uploads)
            self.assertEqual(events, names)
            with patch.object(release, "call", return_value=draft), patch.object(release, "download_sha256", return_value="wrong"):
                with self.assertRaises(ValueError):
                    release.verify_draft("unit-test-token", "owner/repo", "v1.2.3", 10, uploads)
            for bad in [{**draft, "draft": False}, {**draft, "assets": assets[:-1]}]:
                with patch.object(release, "call", return_value=bad):
                    with self.assertRaises(ValueError):
                        release.verify_draft("unit-test-token", "owner/repo", "v1.2.3", 10, uploads)

    def test_asset_redirect_never_forwards_the_job_token(self):
        location = "https://release-assets.githubusercontent.com/test?signature=unit-test"
        error = release.urllib.error.HTTPError("https://api.github.com/test", 302, "Found", {"Location": location}, None)
        with patch.object(release.urllib.request, "build_opener") as opener, patch.object(release.urllib.request, "urlopen", return_value=io.BytesIO(b"downloaded")) as fetch:
            opener.return_value.open.side_effect = error
            got = release.download_sha256("unit-test-token", "owner/repo", {"id": 1, "name": "payload"})
            self.assertEqual(got, hashlib.sha256(b"downloaded").hexdigest())
            fetch.assert_called_once_with(location, timeout=300)

    def test_asset_redirect_outside_github_is_refused(self):
        error = release.urllib.error.HTTPError("https://api.github.com/test", 302, "Found", {"Location": "https://example.com/download"}, None)
        with patch.object(release.urllib.request, "build_opener") as opener:
            opener.return_value.open.side_effect = error
            with self.assertRaises(ValueError):
                release.download_sha256("unit-test-token", "owner/repo", {"id": 1, "name": "payload"})

    def test_verification_failure_never_reaches_the_publish_patch(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "payload"
            path.write_bytes(b"test")
            def call(token, method, url, body=None, **kwargs):
                self.assertNotEqual(method, "PATCH", "failed draft was made public")
                if method == "GET":
                    return []
                if url.endswith("/releases"):
                    return {"id": 10, "html_url": "https://github.com/owner/repo/releases/10", "upload_url": "https://uploads.github.com/test{?name}"}
                return {"id": 1, "name": "payload"}
            with patch.object(release, "call", side_effect=call), patch.object(release, "verify_draft", side_effect=ValueError("corrupt asset")), patch("sys.argv", ["publish-release.py", "--repo", "owner/repo", "--tag", "v1.2.3", "--token", "unit-test-token", str(path)]):
                with self.assertRaises(ValueError):
                    release.main()


if __name__ == "__main__":
    unittest.main()
