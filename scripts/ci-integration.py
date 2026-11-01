#!/usr/bin/env python3
"""Exhaustive, disjoint CI shards; one thread per disposable Docker host.

Run every non-ignored integration test normally, then the original eight log-audit suites
at trace level. Check private logs before printing them. Full trace changes the workload of
timing-sensitive tests, so it is deliberately NOT enabled for the ordinary pass.
No owner credentials or live-server configuration are read.
"""
import argparse
import hashlib
import os
from pathlib import Path
import re
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parent.parent
CARGO = ["cargo", "test", "--manifest-path", str(ROOT / "src-tauri/Cargo.toml"),
         "--features", "integration", "--test", "integration"]
TRACE_SUITES = {"upload_live", "viewers_live", "hls_live", "limits_live", "deploy_clean", "deploy_versions", "deploy_resume", "diag_live"}


def trace_tests(names):
    return [name for name in names if name.split("::", 1)[0] in TRACE_SUITES]


def test_names(listing):
    names = re.findall(r"^(.+): test$", listing, re.MULTILINE)
    if not names:
        raise ValueError("No integration tests found in libtest's listing")
    return names


def partition(names, shard, shards):
    if shards < 1 or not 0 <= shard < shards:
        raise ValueError("Invalid shard index/count")
    if not names or len(names) != len(set(names)):
        raise ValueError("Empty or duplicate test inventory")
    return sorted(name for name in names
                  if int.from_bytes(hashlib.sha256(name.encode()).digest()[:8], "big") % shards == shard)


def check_result(log, selected):
    result = re.search(r"test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored", log)
    if not result:
        raise ValueError("Integration tests failed or left no result")
    passed, failed, ignored = map(int, result.groups())
    if failed or not passed or passed + ignored != selected:
        raise ValueError(f"Incomplete shard: {passed} passed, {ignored} ignored, {selected} selected")


def check_secrets(log, key, passphrase):
    material = [line for line in key.splitlines()[2:6] if len(line) >= 12]
    if not material:
        raise ValueError("No test key material: secret check was NOT performed")
    needles = material + ["BEGIN OPENSSH PRIVATE KEY", "BEGIN RSA PRIVATE KEY", passphrase]
    if any(needle and needle in log for needle in needles):
        raise ValueError("A SECRET WAS FOUND IN THE TRACE LOG; log withheld")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--shard", type=int, required=True)
    parser.add_argument("--shards", type=int, required=True)
    args = parser.parse_args()
    listing = subprocess.run(CARGO + ["--", "--list", "--format", "terse"],
                             check=True, stdout=subprocess.PIPE, text=True, timeout=1800)
    names = test_names(listing.stdout)
    ignored_listing = subprocess.run(CARGO + ["--", "--list", "--ignored", "--format", "terse"],
                                     check=True, stdout=subprocess.PIPE, text=True, timeout=1800)
    ignored = set(re.findall(r"^(.+): test$", ignored_listing.stdout, re.MULTILINE))
    names = [name for name in names if name not in ignored]
    print(f"Ignored live-server/measurement tests: {len(ignored)} (not run)", flush=True)
    groups = [partition(names, i, args.shards) for i in range(args.shards)]
    # Assert the runtime inventory, not just the algorithm's unit test.
    if sorted(sum(groups, [])) != sorted(names):
        raise ValueError("Shard inventory is not exhaustive and disjoint")
    selected = partition(names, args.shard, args.shards)
    if not selected:
        raise ValueError("Empty shard: refusing to report a vacuous pass")
    print(f"Inventory: {len(names)} tests; shard sizes: {[len(g) for g in groups]}", flush=True)
    print(f"Shard {args.shard}: {len(selected)} non-ignored tests selected", flush=True)
    for name in selected:
        print(f"  {name}", flush=True)
    run_checked(selected, trace=False)
    audited = trace_tests(selected)
    if audited:
        run_checked(audited, trace=True)
    else:
        print("No trace-audit tests assigned to this shard", flush=True)


def run_checked(selected, trace):
    with tempfile.TemporaryFile(mode="w+", encoding="utf-8") as output:
        result = subprocess.run(CARGO + ["--", "--test-threads=1", "--nocapture", "--exact"] + selected,
                                env={**os.environ, "VRCAST_LOG": "trace" if trace else "info"},
                                stdout=output, stderr=subprocess.STDOUT, timeout=2400)
        output.seek(0)
        log = output.read()
    key = (ROOT / "src-tauri/tests/fixtures/encrypted_ed25519.key").read_text()
    source = (ROOT / "src-tauri/tests/support/test_key.rs").read_text(encoding="utf-8")
    phrase = re.search(r'PASSPHRASE: &str = "([^"]+)"', source)
    if not phrase:
        raise ValueError("Cannot find the test key's passphrase constant")
    check_secrets(log, key, phrase[1])
    # Only after scanning: never publish an unexamined trace log as an artifact or failure tail.
    print("\n".join(log.splitlines()[-100:]), flush=True)
    if result.returncode:
        lines = log.splitlines()
        for i, line in enumerate(lines):
            if "panicked at" in line:
                print("\n".join(lines[max(0, i - 1):i + 12]), flush=True)
        raise ValueError(f"Integration process exited {result.returncode}")
    check_result(log, len(selected))
    if trace and len(log.splitlines()) < 50:
        raise ValueError("Suspiciously short trace log; secret check was not exercised")
    print(f"No secrets in {len(log.splitlines())} log lines (trace={trace}); every selected test accounted for")


if __name__ == "__main__":
    main()
