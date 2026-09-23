//! T196 — cutting variants into segments **on the server**, as pure logic.
//!
//! What is here is the script that does the cutting, the reading of what it reports, and
//! the rules about how it cuts. Getting it onto a server and watching it is
//! [`crate::server::hls_package`].
//!
//! Ported from `.claude/skills/vrcast-hls/scripts/package-hls.sh` (constitution VI), with
//! one deliberate difference: **the script does not build the master playlist.** The shell
//! version had to, being a shell script; here that arithmetic already exists, measured and
//! tested, in [`super::hls_master`]. Two copies of the peak calculation would drift, and
//! the drift would show up as a player refusing a variant nobody could explain.

use serde::{Deserialize, Serialize};

/// How long one segment is, in seconds.
///
/// **Four, and it was arrived at rather than chosen.** Six gave viewers freezes when the
/// quality changed; two produced so many files that the overhead of asking for each one
/// showed. Four also divides evenly by the keyframe distance every variant is encoded
/// with, which is what lets a player change quality at a segment boundary at all.
pub const SEGMENT_SECONDS: u32 = 4;

/// What the segments are wrapped in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Container {
    /// The classic transport stream. H.264 only.
    Ts,
    /// Fragmented MP4: an `init.mp4` and `.m4s` pieces.
    ///
    /// **Not a preference.** Neither HEVC nor AV1 can be wrapped in a transport stream at
    /// all, so for those it is this or nothing.
    Fmp4,
}

impl Container {
    /// What a segment of this kind is called.
    pub fn extension(&self) -> &'static str {
        match self {
            Self::Ts => "ts",
            Self::Fmp4 => "m4s",
        }
    }
}

/// Which wrapper this codec needs.
pub fn container_for(codec: &str) -> Container {
    match codec.to_ascii_lowercase().as_str() {
        "hevc" | "h265" | "av1" => Container::Fmp4,
        _ => Container::Ts,
    }
}

/// One variant waiting to be cut.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToCut {
    /// The directory the segments go in, under the media's own — `v10`, `v22`.
    pub sub: String,
    /// The prepared file, already on the server, named relative to the serving directory.
    pub file: String,
}

/// How the cutting is getting on.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Progress {
    /// The variants finished so far, in the order they finished.
    pub cut: Vec<String>,
    /// Every variant is done and the script said so.
    pub all_done: bool,
    /// What went wrong, when something did.
    pub failed: Option<String>,
}

/// The marker the script prints when it has finished everything.
///
/// A marker rather than "the process is gone": a process can be gone because it was killed
/// halfway, and the difference decides whether a person is shown a ladder or an apology.
pub const ALL_DONE: &str = "VRCAST_HLS_ALL_DONE";

/// The prefix a finished variant is announced with.
const CUT: &str = "VRCAST_HLS_CUT ";

/// The prefix a failure is announced with.
const FAILED: &str = "VRCAST_HLS_FAILED ";

/// Read the script's log.
pub fn read_log(text: &str) -> Progress {
    let mut progress = Progress::default();
    for line in text.lines() {
        let line = line.trim();
        if let Some(sub) = line.strip_prefix(CUT) {
            progress.cut.push(sub.trim().to_owned());
        } else if let Some(why) = line.strip_prefix(FAILED) {
            progress.failed = Some(why.trim().to_owned());
        } else if line == ALL_DONE {
            progress.all_done = true;
        }
    }
    progress
}

/// What a variant turned out to be, as the server saw it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CutFacts {
    pub sub: String,
    pub width: u32,
    pub height: u32,
    /// The frame rate as written, e.g. `23.976`. Kept as text because that is what goes
    /// into the master playlist and rounding it there would be a lie about the material.
    pub frame_rate: String,
    /// The **actual** level of this variant, as the encoder wrote it into the file.
    pub level: String,
    pub codec: String,
    /// Every segment, in playlist order.
    pub segments: Vec<super::hls_master::Segment>,
}

/// Read the facts a variant's cutting left behind.
///
/// The format is deliberately dull — one `key=value` line for the variant, then one line
/// per segment — because it is written by shell and read by Rust, and anything cleverer
/// would be a parser on both sides.
pub fn read_facts(text: &str) -> Result<CutFacts, FactsProblem> {
    let mut sub = None;
    let mut width = None;
    let mut height = None;
    let mut frame_rate = None;
    let mut level = None;
    let mut codec = None;
    let mut segments = Vec::new();

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("seg ") {
            let mut parts = rest.split_whitespace();
            let (Some(duration), Some(bytes)) = (parts.next(), parts.next()) else {
                continue;
            };
            let (Ok(duration_s), Ok(bytes)) = (duration.parse::<f64>(), bytes.parse::<u64>())
            else {
                continue;
            };
            segments.push(super::hls_master::Segment { duration_s, bytes });
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key {
            "sub" => sub = Some(value.to_owned()),
            "width" => width = value.parse().ok(),
            "height" => height = value.parse().ok(),
            "fps" => frame_rate = Some(value.to_owned()),
            "level" => level = Some(value.to_owned()),
            "codec" => codec = Some(value.to_owned()),
            _ => {}
        }
    }

    Ok(CutFacts {
        sub: sub.ok_or(FactsProblem::Incomplete("sub"))?,
        width: width.ok_or(FactsProblem::Incomplete("width"))?,
        height: height.ok_or(FactsProblem::Incomplete("height"))?,
        frame_rate: frame_rate.ok_or(FactsProblem::Incomplete("fps"))?,
        level: level.ok_or(FactsProblem::Incomplete("level"))?,
        codec: codec.ok_or(FactsProblem::Incomplete("codec"))?,
        segments,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum FactsProblem {
    /// The cutting reported nothing about one of the things the master needs.
    ///
    /// Not recoverable by guessing: a master built on a guessed level cuts the lowest rung
    /// off from the weak devices it exists for.
    #[error("the server said nothing about {0} for this variant")]
    Incomplete(&'static str),
}

/// The script that does the cutting, left on the server and run detached.
///
/// **Detached, and that was bought.** It used to run in the foreground of an SSH session,
/// and any break in the connection killed all the work done: on mandoup a flood of SSH
/// brute-force attempts cut the session in the middle of the third variant, two finished
/// variants had to be rescued by hand and the master was left without its third rung.
///
/// Takes: the serving directory, the owner, the media's own directory name, then one
/// `sub=file` pair per variant.
pub fn script() -> &'static str {
    SCRIPT
}

const SCRIPT: &str = r#"set -uo pipefail

# T605 — say who we are before anything else, so the one who started us can check that we
# really are the leader of a process group of our own (and so can be stopped as a group).
# Read from /proc rather than trusted to the launcher's `$!`: util-linux `setsid` forks when
# it is itself a group leader, and `$!` is then the wrong process.
: "${VRCAST_HLS_PGID_FILE:?the launcher must say where the group is recorded}"
read -r VRCAST_STAT < "/proc/$$/stat"
VRCAST_F=(${VRCAST_STAT##*) })
printf '%s %s %s %s\n' "$$" "${VRCAST_F[2]}" "${VRCAST_F[3]}" "${VRCAST_HLS_JOB:-}" \
    > "$VRCAST_HLS_PGID_FILE.part" \
  && mv -f "$VRCAST_HLS_PGID_FILE.part" "$VRCAST_HLS_PGID_FILE" \
  || { echo "VRCAST_HLS_FAILED -: could not record the process group" >&2; exit 1; }

VIDEODIR="$1"; OWNER="$2"; BASE="$3"; shift 3
OUT="$VIDEODIR/$BASE"
mkdir -p "$OUT"

for spec in "$@"; do
  sub="${spec%%=*}"; file="${spec#*=}"
  src="$VIDEODIR/$file"
  if [ ! -f "$src" ]; then
    echo "VRCAST_HLS_FAILED $sub: no such file: $src" >&2
    exit 1
  fi

  # Already cut, and cut whole? Then leave it alone. **Recognised by what is on the server
  # rather than by a note kept here** (FR-048): a note outlives the thing it describes, and
  # a variant declared ready with half its segments missing is worse than one rebuilt.
  if [ -s "$OUT/$sub/stream.m3u8" ] && grep -q ENDLIST "$OUT/$sub/stream.m3u8" && [ -s "$OUT/$sub/.facts" ]; then
    echo "VRCAST_HLS_CUT $sub"
    continue
  fi

  rm -rf "$OUT/$sub"
  mkdir -p "$OUT/$sub"
  vcodec=$(ffprobe -v error -select_streams v:0 -show_entries stream=codec_name -of csv=p=0 "$src" | tr -d '\r')
  read -r W H < <(ffprobe -v error -select_streams v:0 -show_entries stream=width,height -of csv=p=0 "$src" | tr ',' ' ' | tr -d '\r')
  FPS=$(ffprobe -v error -select_streams v:0 -show_entries stream=r_frame_rate -of csv=p=0 "$src" | tr -d '\r' | awk -F/ '{ if ($2>0) printf "%.3f",$1/$2; else printf "%.3f",$1 }')
  LVL=$(ffprobe -v error -select_streams v:0 -show_entries stream=level -of csv=p=0 "$src" | tr -d '\r')

  # HEVC and AV1 cannot be wrapped in a transport stream at all, so they go into fragmented
  # MP4. H.264 keeps the classic segments.
  #
  # `-nostdin` is not optional: without it ffmpeg reads the rest of THIS script as its own
  # input and everything below silently stops happening.
  if [ "$vcodec" = "hevc" ] || [ "$vcodec" = "av1" ]; then
    ffmpeg -nostdin -y -loglevel error -i "$src" -map 0:v:0 -map 0:a:0 -c copy \
      -f hls -hls_time SEGSECONDS -hls_playlist_type vod \
      -hls_segment_type fmp4 -hls_fmp4_init_filename init.mp4 \
      -hls_flags independent_segments \
      -hls_segment_filename "$OUT/$sub/seg_%05d.m4s" "$OUT/$sub/stream.m3u8" \
      || { echo "VRCAST_HLS_FAILED $sub: the cutting would not run" >&2; exit 1; }
  else
    ffmpeg -nostdin -y -loglevel error -i "$src" -map 0:v:0 -map 0:a:0 -c copy \
      -f hls -hls_time SEGSECONDS -hls_playlist_type vod -hls_segment_type mpegts \
      -hls_flags independent_segments \
      -hls_segment_filename "$OUT/$sub/seg_%05d.ts" "$OUT/$sub/stream.m3u8" \
      || { echo "VRCAST_HLS_FAILED $sub: the cutting would not run" >&2; exit 1; }
  fi

  # What the variant turned out to be, for the master to be built from — by the application,
  # which already knows how, rather than by this script.
  {
    echo "sub=$sub"
    echo "width=$W"
    echo "height=$H"
    echo "fps=$FPS"
    echo "level=$LVL"
    echo "codec=$vcodec"
    dur=""
    while IFS= read -r line; do
      case "$line" in
        '#EXTINF:'*) dur="${line#*:}"; dur="${dur%,*}" ;;
        '#'*) ;;
        '') ;;
        *) [ -n "$dur" ] && echo "seg $dur $(stat -c %s "$OUT/$sub/$line")" ;;
      esac
    done < "$OUT/$sub/stream.m3u8"
  } > "$OUT/$sub/.facts"

  echo "VRCAST_HLS_CUT $sub"
done

chown -R "$OWNER" "$OUT" 2>/dev/null || true
find "$OUT" -type d -exec chmod 755 {} \;
find "$OUT" -type f -exec chmod 644 {} \;
echo "VRCAST_HLS_ALL_DONE"
"#;

/// The script with its one substitution made.
pub fn script_text() -> String {
    SCRIPT.replace("SEGSECONDS", &SEGMENT_SECONDS.to_string())
}

// ---------- T605: the process group the cutting runs in ----------

/// The environment variable every process of one cutting carries.
///
/// **Why a mark in the environment and not a pattern on the command line.** A command line
/// names the wrapper script and nothing else: the `ffmpeg` it starts has no trace of the
/// script in its own, and once the wrapper is dead (killed on its own, or gone for any other
/// reason) a search by command line finds nothing while `ffmpeg` goes on writing. The
/// environment is inherited by every child — `ffprobe`, `ffmpeg`, the lot — and is readable
/// from `/proc/<pid>/environ` by the same user. It also answers the question of a stale
/// record: a process group whose number has been handed out again holds none of our marks,
/// and a group without our mark is treated as "already gone", never signalled.
pub const JOB_VAR: &str = "VRCAST_HLS_JOB";

/// The environment variable that tells the wrapper where to record its group.
pub const PGID_FILE_VAR: &str = "VRCAST_HLS_PGID_FILE";

/// One pass over `/proc`, shared by every script below.
///
/// **Bash builtins only — no `grep`, `ps`, `pgrep` or `awk`.** Every one of those is a
/// program that can be missing, replaced or broken on somebody's server, and each of them
/// fails the same dangerous way here: it finds nothing, and "nothing found" reads as
/// "nothing is running". So the scan also looks for **itself**: a pass that did not see its
/// own process could not read `/proc` at all, and says so (`SEEN_SELF=0`) rather than
/// passing for an empty answer.
///
/// Fills `MARKED` with `pid:pgid` of every live process whose environment carries
/// `VRCAST_HLS_JOB=$WANT…` (a prefix: `base:` means any cutting of that directory,
/// `base:uuid` means one start), and `ALL` with `pid:pgid` of every live process. A zombie
/// is not live — it executes nothing and writes nothing, and in a container with no init to
/// reap it it may stand there for as long as the container does.
///
/// `SEEN_SELF` is raised only when the scan read **its own environment** and found the
/// sentinel it was started with (`VRCAST_HLS_SELFCHECK=1`, set by the caller). That proves
/// the whole chain the answer rests on — `/proc` is there, `stat` parses, `mapfile -d ''`
/// works on this bash (4.4 or later) — on the one process certain to be there. Without it,
/// an old bash failing on `mapfile -d` would find no mark anywhere and answer "nothing is
/// running" about a cutting that is.
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
        "VRCAST_HLS_JOB=$WANT"*) MARKED+=("$p:${f[2]}") ;;
      esac
    done
  done
}
"#;

/// Is anything of ours alive? `$1` is the mark to look for (see [`SCAN`]).
///
/// Prints `VRCAST_RUNNING yes`, `no` or `unreadable`.
pub fn probe_script() -> String {
    format!(
        "set -u\nWANT=\"$1\"\n{SCAN}\nscan\n\
         if [ \"$SEEN_SELF\" != 1 ]; then echo 'VRCAST_RUNNING unreadable'; \
         elif [ ${{#MARKED[@]}} -gt 0 ]; then echo 'VRCAST_RUNNING yes'; \
         else echo 'VRCAST_RUNNING no'; fi\n"
    )
}

/// Start the cutting in a session and group of its own, and report what it recorded.
///
/// Arguments: base, job, script, log, group record file, then the script's own arguments.
///
/// Refuses (`VRCAST_HLS_BUSY`) when a cutting of the same directory is still alive —
/// whoever started it. Two scripts cutting into one directory undo each other's work, and
/// the live one may be another instance's, which is not ours to stop.
pub fn launch_script() -> String {
    format!(
        r#"set -u
BASE="$1"; JOB="$2"; SCRIPT="$3"; LOG="$4"; PGIDF="$5"; shift 5
WANT="$BASE:"
{SCAN}
scan
if [ "$SEEN_SELF" != 1 ]; then echo VRCAST_HLS_UNREADABLE; exit 0; fi
if [ ${{#MARKED[@]}} -gt 0 ]; then echo VRCAST_HLS_BUSY; exit 0; fi
rm -f -- "$PGIDF" "$PGIDF.part"
read -r st < "/proc/$$/stat"; f=(${{st##*) }})
echo "VRCAST_HLS_LAUNCHER ${{f[2]}}"
VRCAST_HLS_JOB="$JOB" VRCAST_HLS_PGID_FILE="$PGIDF" \
  setsid nohup bash "$SCRIPT" "$@" > "$LOG" 2>&1 < /dev/null &
i=0
while [ ! -s "$PGIDF" ] && [ "$i" -lt {RECORD_TICKS} ]; do sleep 0.05; i=$((i + 1)); done
if [ -s "$PGIDF" ]; then read -r rec < "$PGIDF"; echo "VRCAST_HLS_GROUP $rec"; fi
exit 0
"#
    )
}

/// How long the launcher waits for the wrapper's record, in 50 ms ticks: five seconds.
///
/// Measured at ~55 ms in the test container; the margin is for a loaded server, where
/// forking a shell can take far longer than it should, and a start judged failed only
/// because the machine was slow would stop a cutting that was fine.
const RECORD_TICKS: u32 = 100;

/// Stop one start's processes and **confirm** they are gone.
///
/// Arguments: the job mark, then how many 100 ms ticks to wait after TERM, then after KILL.
///
/// TERM goes to every group our marked processes are in, and to each marked process by
/// itself (one that left the group is still ours by its mark). Then the scan is repeated
/// until nothing is left; whoever is still there when the wait runs out gets KILL, and the
/// scan is repeated again. "Nothing left" means: no live process with our mark, and no live
/// process in any group that held one during this stop.
///
/// Prints exactly one of `VRCAST_STOP none` (nothing was alive), `term <ms>ms`,
/// `kill <ms>ms`, `alive <pid:pgid…>` or `unreadable`.
///
/// **A group number cannot be handed out again while any member of the group is alive**,
/// which is what makes signalling a remembered group safe for as long as the stop runs:
/// the group is only remembered from a scan that found our mark in it.
pub fn stop_script() -> String {
    format!(
        r#"set -u
WANT="$1"; TERM_TICKS="$2"; KILL_TICKS="$3"
{SCAN}
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
    # Never a group that is not a cutting's own: not init's (1, and 0 would mean "every
    # process of ours" to kill), and not this very shell's — a cutting that never got a
    # group of its own is still reached, one process at a time, by its mark.
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
"#
    )
}

/// What the launcher said about the start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchReport {
    /// A live cutting for the same media directory was already there, so nothing was
    /// started.
    Busy,
    /// Started, and this is what the wrapper recorded about itself.
    Recorded {
        /// The process group of the shell that did the launching.
        launcher_pgid: u32,
        group: GroupRecord,
    },
    /// Started — or at least asked to start — but the wrapper recorded nothing in time.
    NothingRecorded { launcher_pgid: Option<u32> },
}

/// One line of the group record the wrapper writes about itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupRecord {
    pub pid: u32,
    pub pgid: u32,
    pub sid: u32,
    pub job: String,
}

/// Why a start is not trusted.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GroupProblem {
    #[error("the cutting did not record its process group in time")]
    NotRecorded,
    #[error("the recorded group belongs to another start ({found:?}, expected {expected:?})")]
    NotThisJob { found: String, expected: String },
    #[error("the wrapper (pid {pid}) is not the leader of its group ({pgid})")]
    NotLeader { pid: u32, pgid: u32 },
    #[error("the wrapper (pid {pid}) did not get a session of its own (session {sid})")]
    NoSessionOfItsOwn { pid: u32, sid: u32 },
    #[error("the wrapper stayed in the launching shell's own group ({pgid})")]
    LaunchersGroup { pgid: u32 },
}

/// Read what the launch command printed.
pub fn read_launch(text: &str) -> LaunchReport {
    let mut launcher_pgid = None;
    let mut group = None;
    for line in text.lines() {
        let line = line.trim();
        if line == "VRCAST_HLS_BUSY" {
            return LaunchReport::Busy;
        }
        if let Some(rest) = line.strip_prefix("VRCAST_HLS_LAUNCHER ") {
            launcher_pgid = rest.trim().parse().ok();
        } else if let Some(rest) = line.strip_prefix("VRCAST_HLS_GROUP ") {
            group = read_group_record(rest);
        }
    }
    match (launcher_pgid, group) {
        (Some(launcher_pgid), Some(group)) => LaunchReport::Recorded {
            launcher_pgid,
            group,
        },
        (launcher_pgid, _) => LaunchReport::NothingRecorded { launcher_pgid },
    }
}

/// Read the record itself: `pid pgid sid job`.
pub fn read_group_record(text: &str) -> Option<GroupRecord> {
    let mut parts = text.split_whitespace();
    let pid = parts.next()?.parse().ok()?;
    let pgid = parts.next()?.parse().ok()?;
    let sid = parts.next()?.parse().ok()?;
    let job = parts.next()?.to_owned();
    Some(GroupRecord {
        pid,
        pgid,
        sid,
        job,
    })
}

/// Whether the start can be trusted: the wrapper leads a group of its own, in a session of
/// its own, apart from the shell that launched it, and the record is this start's.
///
/// Every condition is one the stop depends on. A wrapper left in the launching shell's
/// group could not be stopped as a group without reaching for that shell's; a record from
/// an earlier start would point the stop at a group that may be anybody's by now.
pub fn check_launch(
    report: &LaunchReport,
    expected_job: &str,
) -> Result<GroupRecord, GroupProblem> {
    let LaunchReport::Recorded {
        launcher_pgid,
        group,
    } = report
    else {
        return Err(GroupProblem::NotRecorded);
    };
    if group.job != expected_job {
        return Err(GroupProblem::NotThisJob {
            found: group.job.clone(),
            expected: expected_job.to_owned(),
        });
    }
    if group.pgid == *launcher_pgid {
        return Err(GroupProblem::LaunchersGroup { pgid: group.pgid });
    }
    if group.pid != group.pgid || group.pid <= 1 {
        return Err(GroupProblem::NotLeader {
            pid: group.pid,
            pgid: group.pgid,
        });
    }
    if group.sid != group.pid {
        return Err(GroupProblem::NoSessionOfItsOwn {
            pid: group.pid,
            sid: group.sid,
        });
    }
    Ok(group.clone())
}

/// How a confirmed stop went.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stopped {
    /// Nothing of ours was alive to begin with.
    AlreadyGone,
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
            Some("term") => Stopped::AfterTerm,
            Some("kill") => Stopped::AfterKill,
            Some("alive") => {
                return StopReport::StillAlive(words.collect::<Vec<_>>().join(" "));
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
