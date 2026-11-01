#!/usr/bin/env python3
"""Exhaustive, disjoint CI shards; one thread per disposable Docker host.

Run every non-ignored integration test normally in disjoint shards. Run the original eight
log-audit suites TOGETHER in an independent trace job: a hash subset may never initialise
the application's trace writer or redaction canary. Check private logs before printing.
Full trace changes timing-sensitive workloads, so it is NOT enabled for the ordinary pass.
No owner credentials or live-server configuration are read.
"""
import argparse
import hashlib
import os
from pathlib import Path
import re
import signal
import shutil
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parent.parent
CARGO = ["cargo", "test", "--manifest-path", str(ROOT / "src-tauri/Cargo.toml"),
         "--features", "integration", "--test", "integration"]
TRACE_SUITES = {"upload_live", "viewers_live", "hls_live", "limits_live", "deploy_clean", "deploy_versions", "deploy_resume", "diag_live"}


def trace_tests(names):
    return [name for name in names if name.split("::", 1)[0] in TRACE_SUITES]


def select_run(names, shard=None, shards=None, trace_only=False):
    if trace_only:
        if shard is not None or shards is not None:
            raise ValueError("The complete trace audit must not be sharded")
        selected = trace_tests(names)
    else:
        if shard is None or shards is None:
            raise ValueError("An ordinary run needs both --shard and --shards")
        selected = partition(names, shard, shards)
    if not selected:
        raise ValueError("Empty selection: refusing to report a vacuous pass")
    return selected, trace_only


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


def prepare_test_key():
    # Isolation means the first test may use password login only. The log audit must not
    # depend on a later test happening to create its canary. Match support/test_key.rs.
    fixtures = ROOT / 'src-tauri/tests/fixtures'
    key = fixtures / 'encrypted_ed25519.key'
    public = fixtures / 'docker/encrypted_ed25519.key.pub'
    if key.exists() and public.exists():
        return
    source = (ROOT / 'src-tauri/tests/support/test_key.rs').read_text(encoding='utf-8')
    phrase = re.search(r'PASSPHRASE: &str = "([^"]+)"', source)
    if not phrase:
        raise ValueError('Cannot find test key passphrase')
    generated_public = key.with_suffix(key.suffix + '.pub')
    for path in (key, public, generated_public):
        path.unlink(missing_ok=True)
    try:
        subprocess.run(['ssh-keygen', '-q', '-t', 'ed25519', '-N', phrase[1], '-f', str(key)],
                       check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=30)
    except (subprocess.CalledProcessError, subprocess.TimeoutExpired):
        # Do not let subprocess's exception echo passphrase argv into the job log.
        raise ValueError('The disposable test key could not be prepared') from None
    shutil.copyfile(generated_public, public)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--shard", type=int)
    parser.add_argument("--shards", type=int)
    parser.add_argument("--trace-only", action="store_true")
    args = parser.parse_args()
    listing = subprocess.run(CARGO + ["--", "--list", "--format", "terse"],
                             check=True, stdout=subprocess.PIPE, text=True, timeout=1800)
    names = test_names(listing.stdout)
    ignored_listing = subprocess.run(CARGO + ["--", "--list", "--ignored", "--format", "terse"],
                                     check=True, stdout=subprocess.PIPE, text=True, timeout=1800)
    ignored = set(re.findall(r"^(.+): test$", ignored_listing.stdout, re.MULTILINE))
    names = [name for name in names if name not in ignored]
    print(f"Ignored live-server/measurement tests: {len(ignored)} (not run)", flush=True)
    selected, trace = select_run(names, args.shard, args.shards, args.trace_only)
    if not trace:
        groups = [partition(names, i, args.shards) for i in range(args.shards)]
        # Assert the runtime inventory, not just the algorithm's unit test.
        if sorted(sum(groups, [])) != sorted(names):
            raise ValueError("Shard inventory is not exhaustive and disjoint")
        print(f"Inventory: {len(names)} tests; shard sizes: {[len(g) for g in groups]}", flush=True)
    print(f"{'Complete trace audit' if trace else f'Shard {args.shard}'}: {len(selected)} non-ignored tests selected", flush=True)
    for name in selected:
        print(f"  {name}", flush=True)
    prepare_test_key()
    run_checked(selected, trace=trace)


def run_checked(selected, trace):
    # The original trace audit intentionally shares a process. Ordinary tests do not:
    # one hung test must be named, not swallow every later test and the private log.
    if trace:
        return run_group(selected, trace=True, timeout=2400)
    started = time.monotonic()
    failures = []
    for name in selected:
        print(f"Starting isolated integration test: {name}", flush=True)
        try:
            remaining = 2400 - (time.monotonic() - started)
            if remaining <= 0:
                raise ValueError("The shard's 2400s execution budget was exhausted")
            run_group([name], trace=False, timeout=min(1000, remaining))
        except (ValueError, subprocess.TimeoutExpired) as error:
            failures.append(name)
            print(f"FAILED {name}: {error}", flush=True)
    if failures:
        raise ValueError(f"Integration failures ({len(failures)}): {failures}")
    print(f"Every selected test accounted for: {len(selected)} passed", flush=True)


def run_group(selected, trace, timeout):
    timed_out = False
    with tempfile.TemporaryFile(mode="w+", encoding="utf-8") as output:
        result = subprocess.Popen(CARGO + ["--", "--test-threads=1", "--nocapture", "--exact"] + selected,
                                  env={**os.environ, "VRCAST_LOG": "trace" if trace else "info"},
                                  stdout=output, stderr=subprocess.STDOUT,
                                  start_new_session=os.name != "nt")
        try:
            result.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            timed_out = True
            if os.name == "nt":
                subprocess.run(["taskkill", "/PID", str(result.pid), "/T", "/F"], check=False,
                               stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            else:
                os.killpg(result.pid, signal.SIGKILL)
            result.wait()
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
    if timed_out:
        raise ValueError(f"Timed out after {timeout:.0f}s: {selected}; process tree stopped")
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
