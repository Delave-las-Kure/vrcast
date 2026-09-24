//! T605, generalised by T609 — processes on a server found by a **mark in their environment**,
//! and stopped with a confirmation rather than a hope.
//!
//! Two kinds of work use it: the cutting of a ladder (`VRCAST_HLS_JOB`, T605) and every
//! remote command of a deployment or an upgrade (`VRCAST_DEPLOY_RUN`, T609). What differs
//! between them is only the name of the variable; the scan, the stop and the reading of its
//! answer are one piece of code, so a fix to one is a fix to both.
//!
//! **Why a mark in the environment and not a pattern on the command line.** A command line
//! names the wrapper and nothing else: the `ffmpeg` or the `dpkg` it starts has no trace of
//! it in its own, and once the wrapper is dead a search by command line finds nothing while
//! the child goes on working. The environment is inherited by every child and is readable
//! from `/proc/<pid>/environ` by the same user. A process that must outlive the work on
//! purpose drops the mark explicitly (`env -u`) — the deployment's SSH undo timer does.

/// One pass over `/proc`, shared by every script built here. `MARKVAR` is replaced by the
/// name of the variable (see [`scan_for`]).
///
/// **Bash builtins only — no `grep`, `ps`, `pgrep` or `awk`.** Every one of those is a
/// program that can be missing, replaced or broken on somebody's server, and each of them
/// fails the same dangerous way here: it finds nothing, and "nothing found" reads as
/// "nothing is running". So the scan also looks for **itself**: a pass that did not see its
/// own process could not read `/proc` at all, and says so (`SEEN_SELF=0`) rather than
/// passing for an empty answer.
///
/// Fills `MARKED` with `pid:pgid` of every live process whose environment carries
/// `MARKVAR=$WANT…` (a prefix), and `ALL` with `pid:pgid` of every live process. A zombie is
/// not live — it executes nothing and writes nothing, and in a container with no init to
/// reap it it may stand there for as long as the container does.
///
/// `SEEN_SELF` is raised only when the scan read **its own environment** and found the
/// sentinel it was started with (`VRCAST_HLS_SELFCHECK=1`, set by the caller). That proves
/// the whole chain the answer rests on — `/proc` is there, `stat` parses, `mapfile -d ''`
/// works on this bash (4.4 or later) — on the one process certain to be there. The sentinel
/// keeps its T605 name for both kinds of work: it is a check of the scan, not of the cutting.
const SCAN: &str = r#"
self=$$
scan() {
  MARKED=(); ALL=(); SEEN_SELF=0
  local d p st e
  local -a f env
  for d in /proc/[0-9]*; do
    p=${d#/proc/}
    read -r st 2>/dev/null < "$d/stat" || continue
    f=(${st##*) })
    case "${f[0]}" in Z|X|x) continue ;; esac
    ALL+=("$p:${f[2]}")
    env=()
    mapfile -d '' -t env 2>/dev/null < "$d/environ" || continue
    for e in "${env[@]}"; do
      case "$e" in
        "VRCAST_HLS_SELFCHECK=1") [ "$p" = "$self" ] && SEEN_SELF=1 ;;
        "MARKVAR=$WANT"*) MARKED+=("$p:${f[2]}") ;;
      esac
    done
  done
}
"#;

/// The sentinel the scan checks itself against; the caller must set it in the environment
/// of the shell that runs any script built here.
pub const SELFCHECK: &str = "VRCAST_HLS_SELFCHECK=1";

/// The scan, looking for the variable `var`.
///
/// `var` is one of this program's own constants and never anything a person typed; it is
/// checked anyway, because it goes into a script as it is.
pub fn scan_for(var: &str) -> String {
    assert!(
        !var.is_empty() && var.chars().all(|c| c.is_ascii_uppercase() || c == '_'),
        "a mark's variable must be a plain upper-case name: {var:?}"
    );
    SCAN.replace("MARKVAR", var)
}

/// Is anything carrying the mark alive? `$1` is the mark to look for (a prefix).
///
/// Prints `VRCAST_RUNNING yes`, `no` or `unreadable`.
pub fn probe_script(var: &str) -> String {
    format!(
        "set -u\nWANT=\"$1\"\n{scan}\nscan\n\
         if [ \"$SEEN_SELF\" != 1 ]; then echo 'VRCAST_RUNNING unreadable'; \
         elif [ ${{#MARKED[@]}} -gt 0 ]; then echo 'VRCAST_RUNNING yes'; \
         else echo 'VRCAST_RUNNING no'; fi\n",
        scan = scan_for(var)
    )
}

/// Stop every process carrying the mark and **confirm** they are gone.
///
/// Arguments: the mark (a prefix), how many 100 ms ticks to wait after TERM, then after
/// KILL, and — optional, T609 — how many **seconds** to wait first for the marked processes
/// to end on their own (default 0), and what to do if they have not by then: `signal`
/// (default) or `wait`, which signals nothing and answers `running`.
///
/// The first wait is what lets a deployment be stopped without tearing `dpkg` in half: when
/// the stop is asked through a fresh connection because the old one died while a command was
/// still running, that command is given the rest of its own time to finish before it is
/// treated as hung. The cutting passes nothing and gets none.
///
/// TERM goes to every group our marked processes are in, and to each marked process by
/// itself (one that left the group is still ours by its mark). Then the scan is repeated
/// until nothing is left; whoever is still there when the wait runs out gets KILL, and the
/// scan is repeated again. "Nothing left" means: no live process with our mark, and no live
/// process in any group that held one during this stop.
///
/// Prints exactly one of `VRCAST_STOP none` (nothing was alive), `ended <ms>ms` (alive, and
/// ended on its own within the first wait), `term <ms>ms`, `kill <ms>ms`,
/// `running <pid:pgid…>` (still alive after the wait, and told not to signal),
/// `alive <pid:pgid…>` or `unreadable`.
///
/// **A group number cannot be handed out again while any member of the group is alive**,
/// which is what makes signalling a remembered group safe for as long as the stop runs:
/// the group is only remembered from a scan that found our mark in it.
pub fn stop_script(var: &str) -> String {
    format!(
        r#"set -u
WANT="$1"; TERM_TICKS="$2"; KILL_TICKS="$3"; GRACE_S="${{4:-0}}"; AFTER="${{5:-signal}}"
{scan}
groups=" "
read -r st < "/proc/$$/stat"; f=(${{st##*) }}); own_pgid=${{f[2]}}
t0=${{EPOCHREALTIME:-0}}; t0=${{t0//[.,]/}}
ms() {{ local now=${{EPOCHREALTIME:-0}}; now=${{now//[.,]/}}; echo $(( (now - t0) / 1000 )); }}
alive() {{
  scan
  if [ "$SEEN_SELF" != 1 ]; then echo 'VRCAST_STOP unreadable'; exit 0; fi
  local x g
  for x in "${{MARKED[@]}}"; do
    g=${{x#*:}}
    # Never a group that is not the work's own: not init's (1, and 0 would mean "every
    # process of ours" to kill), and not this very shell's — work that never got a group of
    # its own is still reached, one process at a time, by its mark.
    [ "$g" -le 1 ] && continue
    [ "$g" = "$own_pgid" ] && continue
    case "$groups" in *" $g "*) ;; *) groups+="$g " ;; esac
  done
  [ ${{#MARKED[@]}} -gt 0 ] && return 0
  for x in "${{ALL[@]}}"; do
    case "$groups" in *" ${{x#*:}} "*) return 0 ;; esac
  done
  return 1
}}
signal() {{
  local g x
  for g in $groups; do kill -s "$1" -- "-$g" 2>/dev/null; done
  for x in "${{MARKED[@]}}"; do [ "${{x%%:*}}" != "$self" ] && kill -s "$1" "${{x%%:*}}" 2>/dev/null; done
}}
alive || {{ echo 'VRCAST_STOP none'; exit 0; }}
i=0
while [ "$i" -lt "$GRACE_S" ]; do
  sleep 1
  alive || {{ echo "VRCAST_STOP ended $(ms)ms"; exit 0; }}
  i=$((i + 1))
done
if [ "$AFTER" = wait ]; then echo "VRCAST_STOP running ${{MARKED[*]}}"; exit 0; fi
signal TERM
i=0
while [ "$i" -lt "$TERM_TICKS" ]; do
  sleep 0.1
  alive || {{ echo "VRCAST_STOP term $(ms)ms"; exit 0; }}
  i=$((i + 1))
done
alive && signal KILL
i=0
while [ "$i" -lt "$KILL_TICKS" ]; do
  sleep 0.1
  alive || {{ echo "VRCAST_STOP kill $(ms)ms"; exit 0; }}
  i=$((i + 1))
done
alive || {{ echo "VRCAST_STOP kill $(ms)ms"; exit 0; }}
echo "VRCAST_STOP alive ${{MARKED[*]}} groups:$groups"
"#,
        scan = scan_for(var)
    )
}

/// How a confirmed stop went.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stopped {
    /// Nothing of ours was alive to begin with.
    AlreadyGone,
    /// Alive when asked, and ended on its own within the wait before any signal (T609).
    EndedOnItsOwn,
    /// Ended on the polite signal.
    AfterTerm,
    /// Had to be killed.
    AfterKill,
}

/// What the stop command printed, read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopReport {
    Confirmed {
        how: Stopped,
        elapsed_ms: Option<u64>,
    },
    /// Still alive after the kill — these, as `pid` lists.
    StillAlive(String),
    /// Still alive after the wait, and deliberately not signalled — these (T609).
    StillRunning(String),
    /// Nothing that could be read as an answer.
    Unreadable(String),
}

/// Read the stop command's answer. Anything that is not one of its own words is not a
/// confirmation — the whole point of the answer is that silence does not count as one.
pub fn read_stop(text: &str) -> StopReport {
    for line in text.lines() {
        let Some(rest) = line.trim().strip_prefix("VRCAST_STOP ") else {
            continue;
        };
        let mut words = rest.split_whitespace();
        let how = match words.next() {
            Some("none") => Stopped::AlreadyGone,
            Some("ended") => Stopped::EndedOnItsOwn,
            Some("term") => Stopped::AfterTerm,
            Some("kill") => Stopped::AfterKill,
            Some("alive") => {
                return StopReport::StillAlive(words.collect::<Vec<_>>().join(" "));
            }
            Some("running") => {
                return StopReport::StillRunning(words.collect::<Vec<_>>().join(" "));
            }
            _ => continue,
        };
        let elapsed_ms = words
            .next()
            .and_then(|w| w.strip_suffix("ms"))
            .and_then(|w| w.parse().ok());
        return StopReport::Confirmed { how, elapsed_ms };
    }
    StopReport::Unreadable(text.trim().chars().take(200).collect())
}
