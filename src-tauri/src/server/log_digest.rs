//! T314 — reading a stretch of the serving's log off the server.
//!
//! **Never the whole file.** The log is capped at 250 MB by the deployment, and dragging that
//! across somebody's home connection to count status codes would take longer than the
//! complaint that prompted it. Worse, reading it end to end on the server means 250 MB of disk
//! reads competing with the very serving being investigated — the diagnosis would itself
//! become a cause of what it was called in to explain.
//!
//! So it is read **from the end, in doubling steps**, and the walking happens on the server:
//! `tail -n N` seeks from the end rather than reading forward, so asking for the last few
//! thousand lines of a huge file is cheap. The step doubles until the oldest line reached is
//! older than the moment asked for, or until the cap.
//!
//! **The cap is never silent.** When it is hit, the answer says so, because a digest that
//! quietly covers the last ten minutes of an hour asked for is a digest that answers a
//! different question than the one put to it.

use time::OffsetDateTime;

use crate::domain::access_log::{parse_line, Request};
use crate::ssh::{Connection, Result};

/// Where the serving writes what it served.
pub use super::viewers::ACCESS_LOG_PATH;

/// How many lines the first step asks for.
pub const FIRST_STEP: usize = 2_000;

/// The most lines that will ever be brought across.
///
/// **A choice.** At roughly 250 bytes a line this is some twelve megabytes — a few seconds on
/// any connection, and enough to cover a whole evening's watching by a room of people. Beyond
/// it the answer says what it could not reach rather than growing without limit.
pub const MOST_LINES: usize = 50_000;

/// A stretch of the log, as it came back.
#[derive(Debug, Clone)]
pub struct Stretch {
    pub requests: Vec<Request>,
    /// Lines that yielded nothing. Counted rather than dropped: a stretch of nothing but
    /// these means the serving is writing something other than what is expected.
    pub unreadable: usize,
    /// Whether the cap was reached, so that what is shown covers less than what was asked
    /// for. Said out loud; see the module note.
    pub reached_the_cap: bool,
    /// The oldest moment actually covered, as far as could be reached.
    pub oldest: Option<OffsetDateTime>,
}

/// What is asked, and how the answer is laid out.
pub fn command(since: OffsetDateTime, until: Option<OffsetDateTime>, log_path: &str) -> String {
    const ASK: &str = r#"
LOG={LOG}
n={FIRST}
capped=no
while :; do
  if [ "$n" -ge {CAP} ]; then n={CAP}; capped=yes; break; fi
  # `tail -n` seeks from the end: asking a huge file for its last few thousand lines does
  # not read the rest of it. This is what makes the walk affordable at all.
  oldest=$(tail -n "$n" "$LOG" 2>/dev/null | head -n 1 | grep -o '"ts":[0-9.]*' | head -n 1 | cut -d: -f2)
  [ -z "$oldest" ] && break
  # Reached far enough back? Compared with awk because shell arithmetic cannot do fractions,
  # and these are unix seconds with a fraction on the end.
  awk -v a="$oldest" -v b="{SINCE}" 'BEGIN{exit !(a<=b)}' && break
  # And if the file simply has no more lines to give, doubling for ever would spin.
  have=$(tail -n "$n" "$LOG" 2>/dev/null | wc -l)
  [ "$have" -lt "$n" ] && break
  n=$((n*2))
done
printf 'capped=%s\n' "$capped"
printf 'fence=%s\n' '{FENCE}'
tail -n "$n" "$LOG" 2>/dev/null | awk -v since="{SINCE}" -v until="{UNTIL}" '
{
  ts = ""
  if (match($0, /"ts":[0-9.]+/)) { ts = substr($0, RSTART + 5, RLENGTH - 5) }
  if (ts == "") { next }
  if (ts + 0 < since + 0) { next }
  if (until != "" && ts + 0 > until + 0) { next }
  print
}'
"#;

    ASK.replace("{LOG}", &super::shell_quote(log_path))
        .replace("{FIRST}", &FIRST_STEP.to_string())
        .replace("{CAP}", &MOST_LINES.to_string())
        .replace("{FENCE}", FENCE)
        .replace("{SINCE}", &unix(since))
        .replace("{UNTIL}", &until.map(unix).unwrap_or_default())
}

/// What separates the header from the lines themselves.
///
/// A fence rather than a line count: the lines are JSON written by something else, and any
/// arithmetic about how many there ought to be is an assumption about a file we do not write.
const FENCE: &str = "--vrcast-log--";

/// Read a stretch of the log.
pub async fn over(
    conn: &Connection,
    since: OffsetDateTime,
    until: Option<OffsetDateTime>,
    log_path: &str,
) -> Result<Stretch> {
    let said = conn.exec(&command(since, until, log_path)).await?;
    Ok(read(&said.stdout))
}

/// Turn the answer into requests.
pub fn read(said: &str) -> Stretch {
    let mut reached_the_cap = false;
    let mut requests = Vec::new();
    let mut unreadable = 0usize;
    let mut past_fence = false;

    for line in said.lines() {
        if !past_fence {
            if let Some(value) = line.strip_prefix("capped=") {
                reached_the_cap = value.trim() == "yes";
            }
            if line.trim().ends_with(FENCE) {
                past_fence = true;
            }
            continue;
        }
        if line.trim().is_empty() {
            continue;
        }
        match parse_line(line) {
            Ok(request) => requests.push(request),
            Err(_) => unreadable += 1,
        }
    }

    let oldest = requests.iter().map(|r| r.at).min();
    Stretch {
        requests,
        unreadable,
        reached_the_cap,
        oldest,
    }
}

/// Unix seconds, as the log writes them.
fn unix(at: OffsetDateTime) -> String {
    format!("{}", at.unix_timestamp())
}
