#!/usr/bin/env bash
# T064 — hunting for secrets in the log (SC-011, constitution, principle IV).
#
# Runs the acceptance check against the live server with the most detailed log the
# application can produce, and searches the output for traces of the private key. The
# detailed log is the whole point: at the info level talkative libraries stay quiet and the
# check would be idle — while at trace they lay out everything they know, including what
# nobody should.
#
# The script NEVER prints the key's contents: it takes a stretch out of it, looks for that in
# the log, and reports only "found" or "not found".
#
# To run it (needs a real server and a server.env beside the project):
#   bash scripts/check-no-secrets-in-logs.sh            # the live server, read-only
#   bash scripts/check-no-secrets-in-logs.sh container  # a throwaway server, AN UPLOAD
#
# The second mode appeared because uploading added paths to the output of its own — the
# connection is held for hours, reconnects, goes over SFTP — and not one of them is touched
# by a check that only reads the library (T128). The live server is neither fit nor needed
# for that: the code is the same, and there is no reason to touch somebody's files.
set -euo pipefail

APP="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ROOT="$(cd "$APP/.." && pwd)"
ENV_FILE="$ROOT/server.env"
MODE="${1:-live}"

# The passphrase of the test key. It must match PASSPHRASE in
# src-tauri/tests/support/test_key.rs: this check looks for exactly that string in the log,
# and a stale value here would search for something that never appears — the check would
# pass while proving nothing.
TEST_KEY_PASSPHRASE="test-passphrase-1234"

if [ "$MODE" = "container" ]; then
  # The throwaway server's key is made on the spot by the tests. It opens nothing anywhere,
  # but it has no place in a log all the same: the rule about secrets knows no difference
  # between a valuable one and a practice one, and what must be checked is exactly what runs.
  KEY="$APP/src-tauri/tests/fixtures/encrypted_ed25519.key"
  if [ ! -f "$KEY" ]; then
    echo "There is no test key. Run any integration test — it will create one." >&2
    exit 2
  fi
elif [ ! -f "$ENV_FILE" ]; then
  echo "There is no $ENV_FILE — this check is meant for a machine with a real server set up." >&2
  echo "To check the upload's output paths: bash $0 container" >&2
  exit 2
else

KEY=$(grep -E "^SSH_KEY=" "$ENV_FILE" | sed -E 's/^SSH_KEY="?([^"#]*)"?.*/\1/' | sed "s|\$HOME|$HOME|" | xargs)
if [ ! -f "$KEY" ]; then
  echo "The key file named in server.env was not found." >&2
  exit 2
fi
fi

LOG="$(mktemp)"
trap 'rm -f "$LOG" "$LOG.needles"' EXIT

# The filters below are test names. They must match the names in
# src-tauri/tests/integration/: a stale filter selects nothing, `cargo test` exits with
# success, and the check reports a clean log it never produced. That is not a warning
# any more — it is checked below, by counting the tests that actually ran.
#
# `cargo test` takes several filters and runs the union of them.
CONTAINER_TESTS="upload_live viewers_live hls_live limits_live"
# Веха D. Развёртывание — единственное место, где приложение **само заводит закрытый ключ**
# (T290a), и он идёт по тем же путям, что и чужой: в хранилище системы, в соединение, в
# отчёты шагов. Ключа этого нет ни в одном файле, так что искать его в журнале построчно
# нечем — зато его заголовок `BEGIN OPENSSH PRIVATE KEY` уже стоит в списке искомого, и
# утёкший ключ принесёт его с собой.
#
# Диагностика тут же: она читает журнал раздачи и адреса зрителей, а FR-057 запрещает
# отдавать их куда бы то ни было — журнал приложения не исключение.
CONTAINER_TESTS="$CONTAINER_TESTS deploy_clean deploy_versions deploy_resume diag_live"
if [ "$MODE" = "container" ]; then
  # Milestone C brought output paths of its own, and none of them is touched by an
  # upload: watching viewers holds a connection open for as long as a screen is,
  # cutting a ladder leaves a script on the server and reads its log back, and a
  # quality limit writes into the serving's own configuration. Every one of those is a
  # place a secret could come out, so every one of them runs here.
  echo "Running the server-side checks against a throwaway server with a detailed log…"
  RUN=(--test-threads=1 --nocapture $CONTAINER_TESTS)
else
  echo "Running the acceptance check with a detailed log…"
  RUN=(--ignored --nocapture the_live_server_read_only)
fi

(
  cd "$APP/src-tauri"
  VRCAST_LOG=trace cargo test --features integration --test integration -- "${RUN[@]}"
) > "$LOG" 2>&1 || {
  echo "The acceptance check did not pass — it is too early to hunt for secrets in a failed run's log." >&2
  tail -20 "$LOG" >&2
  exit 1
}

# The stretches that must not be in the log. The key's body is taken from the middle of the
# file: the first line is the same in every key and proves nothing.
{
  sed -n '3,6p' "$KEY"
  echo "BEGIN OPENSSH PRIVATE KEY"
  echo "BEGIN RSA PRIVATE KEY"
  # In container mode the passphrase is looked for too: it goes down the same paths as a real
  # server password, and its leaking would mean that one leaks as well.
  [ "$MODE" = "container" ] && echo "$TEST_KEY_PASSPHRASE"
} > "$LOG.needles"

found=0
while IFS= read -r needle; do
  [ ${#needle} -lt 12 ] && continue
  if grep -qF "$needle" "$LOG"; then
    echo "A TRACE OF A SECRET WAS FOUND IN THE LOG (a stretch ${#needle} characters long)" >&2
    found=1
  fi
done < "$LOG.needles"

# **How many tests actually ran.** This is the failure the whole file is written
# against: a filter that names a test which has been renamed selects nothing, `cargo
# test` reports success, and this check announces a clean log it never read. Counting
# the log's own line is the only way to tell "nothing was wrong" from "nothing
# happened".
ran=$(grep -oE 'test result: ok\. [0-9]+ passed' "$LOG" | grep -oE '[0-9]+' | head -1)
if [ -z "${ran:-}" ] || [ "$ran" -eq 0 ]; then
  echo "NOT ONE TEST RAN — the filters name tests that no longer exist." >&2
  echo "Filters: ${RUN[*]}" >&2
  echo "The check would otherwise report a clean log it never produced." >&2
  exit 1
fi

# And a check that the log was written at all: an empty log would "pass" while checking
# nothing.
lines=$(wc -l < "$LOG")
if [ "$lines" -lt 50 ]; then
  echo "The log is suspiciously short ($lines lines) — the check most likely ran idle." >&2
  exit 1
fi

if [ "$found" -ne 0 ]; then
  echo "" >&2
  echo "Principle IV is broken: a secret reached the log." >&2
  exit 1
fi

echo "No secrets found in the log ($lines lines, $ran tests, level trace, mode $MODE)."
