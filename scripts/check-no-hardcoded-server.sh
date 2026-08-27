#!/usr/bin/env bash
# T060a — the application must hold not one hardcoded server (FR-004).
#
# The application is handed out to people, each with a server of their own. The author's
# address, domain or path, slipped in while carrying logic over from the skills, is a defect
# that shows up quietly on somebody else's machine.
set -euo pipefail

APP="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$APP"

# The values come from the environment so that the script itself does not become the place
# an address is kept. In CI they are set as secrets; locally, by hand from server.env.
#
# The warning checks the variables themselves rather than the array's length: the condition
# used to stand after the default path was added unconditionally and so never fired — a run
# with no secrets (a fork, or locally) looked like a complete check.
if [ -z "${FORBID_IP:-}" ] && [ -z "${FORBID_DOMAIN:-}" ]; then
  echo "Warning: FORBID_IP and FORBID_DOMAIN are not set — the server's address and domain are NOT checked." >&2
fi

NEEDLES=()
[ -n "${FORBID_IP:-}" ]     && NEEDLES+=("$FORBID_IP")
[ -n "${FORBID_DOMAIN:-}" ] && NEEDLES+=("$FORBID_DOMAIN")
NEEDLES+=("/var/lib/vrcast/videos")   # the default path is allowed ONLY as a default in the settings

# Where to search: the sources AND Tauri's packaging files — a server's address really does
# settle in tauri.conf.json (the CSP) and in capabilities/ (permissions for remote URLs).
SCOPE=(src src-tauri/src src-tauri/tauri.conf.json)
[ -d src-tauri/capabilities ] && SCOPE+=(src-tauri/capabilities)

# The tests, searched for the ADDRESS and the DOMAIN only (T249).
#
# They were outside the scope entirely until milestone D, and until milestone D that was
# harmless: a test pointed at the wrong server spoils a file. Deployment rewrites the SSH
# configuration, turns password logins off and switches the network filter on — pointed at a
# working server it takes the serving down and can lock its owner out. The fixture guards the
# address it hands out (deploy_fixture.rs), and this is the second lock: on the text, so that
# an address cannot get in by another road.
#
# The path needle is deliberately NOT applied here. /var/lib/vrcast/videos is where the
# container serves from, on purpose and in a dozen places — the paths take part in the checks,
# and substituting them would check a different case.
TEST_SCOPE=()
[ -d src-tauri/tests ] && TEST_SCOPE+=(src-tauri/tests)
ADDRESS_NEEDLES=()
[ -n "${FORBID_IP:-}" ]     && ADDRESS_NEEDLES+=("$FORBID_IP")
[ -n "${FORBID_DOMAIN:-}" ] && ADDRESS_NEEDLES+=("$FORBID_DOMAIN")

# The one allowed exception: a line marked "FR-004-ok". It is needed for exactly one case —
# a default value that goes into a new profile and is immediately there for a person to
# edit. The exceptions are COUNTED and printed: a quietly growing list of marks is the same
# hardcoded server, only with permission.
ALLOW_MARK="FR-004-ok"

fail=0
exempted=0
for needle in "${NEEDLES[@]}"; do
  hits="$(grep -rInF "$needle" "${SCOPE[@]}" 2>/dev/null || true)"
  [ -z "$hits" ] && continue

  allowed="$(printf '%s\n' "$hits" | grep -F "$ALLOW_MARK" || true)"
  forbidden="$(printf '%s\n' "$hits" | grep -vF "$ALLOW_MARK" || true)"

  if [ -n "$allowed" ]; then
    exempted=$((exempted + $(printf '%s\n' "$allowed" | grep -c .)))
  fi

  if [ -n "$forbidden" ]; then
    echo "A HARDCODED SERVER WAS FOUND ($needle):" >&2
    echo "$forbidden" >&2
    fail=1
  fi
done

# The tests. No exception mark is honoured here: a default value a person can edit is a
# reasonable thing in the application and has no meaning at all in a test, where the address
# is not offered to anybody — it is simply used.
if [ "${#TEST_SCOPE[@]}" -gt 0 ] && [ "${#ADDRESS_NEEDLES[@]}" -gt 0 ]; then
  for needle in "${ADDRESS_NEEDLES[@]}"; do
    hits="$(grep -rInF "$needle" "${TEST_SCOPE[@]}" 2>/dev/null || true)"
    [ -z "$hits" ] && continue
    echo "THE LIVE SERVER'S ADDRESS WAS FOUND IN THE TESTS ($needle):" >&2
    echo "$hits" >&2
    echo "The checks run against the throwaway stand. Deployment against a working server" >&2
    echo "does not spoil a file — it takes the serving down." >&2
    fail=1
  done
fi

if [ "$fail" -ne 0 ]; then
  echo "" >&2
  echo "FR-004 is broken. A server's address must come only from a person's profile." >&2
  echo "If this is a legitimate default, mark the line \"$ALLOW_MARK\"," >&2
  echo "but remember: every mark lands in the report below and has to be justified." >&2
  exit 1
fi

if [ "$exempted" -gt 0 ]; then
  echo "No hardcoded servers found (marked exceptions: $exempted)."
else
  echo "No hardcoded servers found."
fi
