//! T211, T212, T213, T216 — putting a quality limit on the server, and taking it off again.
//!
//! **The application writes one file and no other** (R-03). The main serving configuration
//! belongs to the person: it may hold things this application knows nothing about, and a
//! mistake in it costs the whole of the serving — including a showing that is happening at
//! that moment. Our rules go into a file the main configuration imports, and that file is
//! the only one ever replaced.
//!
//! **Nothing here is allowed to leave the serving broken** (FR-063). Every step from the
//! checking onwards can put back what was there before, reload, and make sure it works.
//!
//! **One transaction for the whole change** (T603). T600 made the compare-and-swap atomic,
//! but only the swap itself: the shortened descriptions were written before it, the
//! checking, the reload and the viewer's check after it, and a failed check put back a
//! shared `.previous` without asking whose it was. So A put its rules in, B put its own in
//! on top while A was still waiting on the serving, A's check failed, and A's rollback
//! moved back the copy B had kept — of A's own file. B's rule was gone without a word, A's
//! rolled-back rule was in force again, and the generation went backwards. Now every change
//! holds one server-side lock from before it touches anything until after its last check or
//! its rollback — see `Serving::apply`.

use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::domain::limits_conf::{self, Limit};
use crate::domain::slow_master::{self, shorten, slow_master_path};
use crate::ssh::{Connection, SshError};

/// How long the serving is given to answer after a reload.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(20);

/// How long a change waits for another change on the same server to finish (T603).
///
/// Must be longer than the longest a change can hold the lock, or a change queued behind a
/// slow-but-healthy one is turned away for nothing. The longest is a change whose check
/// fails: validate + reload + `ANSWER_TIMEOUT` + rollback + reload + `ANSWER_TIMEOUT`
/// again, plus a dozen SSH round-trips. Measured on the test server (2026-09-23):
/// `caddy validate` and `caddy reload` take 17–18 ms each, and that worst case — a check
/// against an address that never answers, twice — took 40.4 s end to end. Two minutes is
/// room for one such change ahead in the queue with a wide margin for a slow link and a
/// slower machine, and short enough that a person asked to "read again and retry" is not
/// left staring at a spinner for longer than they would believe.
const LOCK_WAIT: Duration = Duration::from_secs(120);

/// How long the server keeps the lock with nothing heard from its holder (T618).
///
/// The lock lives as long as one process on the server does. That process ends the moment
/// our channel to it closes — a client that crashed, or a connection that dropped, frees the
/// lock by itself — and, since T618, also when it has heard nothing from us for this long.
/// The second is for the one case the first does not catch: a connection that is neither
/// alive nor closed, which the server may not notice for hours (the test server's sshd has
/// no `ClientAliveInterval`, and TCP keepalive defaults to two hours).
///
/// **Silence, not age.** T603 bounded the holder with a fixed `timeout 300`, and a change
/// that was still alive and working — one command may run up to `EXEC_CEILING`, 600 s —
/// lost its lock in the middle: another change took it and wrote over the first, or the
/// first one's rollback was refused (`LOST_LOCK`) and its failed rules stayed in force
/// (QA-19 №3). Now the lock is held for exactly as long as the client keeps saying it is
/// alive (`RENEW_EVERY`), however long the change takes, its rollback included.
///
/// **Why 60 s.** It must be below `LOCK_WAIT`, so a change queued behind a connection that
/// went silent still gets the lock within its own wait instead of being turned away. And
/// it must be well above `RENEW_EVERY`: what the server has to see is the gap between two
/// renewals **arriving**, and that is `RENEW_EVERY` plus however much later the second
/// was delivered than the first. On a healthy link that is milliseconds; what stretches it
/// is TCP retransmission over a flapping link, and the renewal queued behind other data on
/// the same connection (an upload's SFTP writes share it, R-04) — seconds to tens of seconds
/// on a slow uplink. 60 s leaves 45 s of such delay before a live change is taken for a
/// dead one, and frees a truly silent one within a minute rather than five.
pub const LOCK_SILENCE: Duration = Duration::from_secs(60);

/// How often the client tells the lock's holder it is still alive (T618).
///
/// A quarter of `LOCK_SILENCE`: three renewals in a row may be lost to delay before the
/// server lets the lock go, and each costs one byte on a channel of its own — nothing next
/// to the viewers' watching or an upload. Shorter would buy nothing (the delay that matters
/// is the link's, not ours); longer would eat into the margin above.
pub const RENEW_EVERY: Duration = Duration::from_secs(15);

/// How long giving the lock back is waited on before being left to the channel's closing.
const RELEASE_WAIT: Duration = Duration::from_secs(15);

/// The mark of the lock's **door**: the process that listens for the renewals, and lives
/// exactly as long as the client is heard from (T603, T628). A step going **forward**
/// starts only while it is there (`guard`), so a step can tell "the lock is held by us, and
/// we are alive" from "the lock is held by somebody" or "a process with the PID we were told
/// now belongs to somebody else".
///
/// Before T628 the mark sat on `flock` itself, and `flock` ended with the silence: the lock
/// went at the same instant as the door closed, whatever this change still had running.
const TXN_ENV: &str = "VRCAST_LIMITS_TXN";

/// The mark of the holder itself — the process `flock` waits on — for as long as this
/// change **holds** the lock (T628): from the moment it is taken until the holder has
/// waited once for every step of the change; then the holder drops it (`exec env -u`, the
/// same PID) and looks once more.
///
/// Putting back (`undo`) goes in under it (`guard_held`) rather than under the door: a
/// change nobody hears from must not go forward, but one whose client comes back while the
/// holder is still waiting for its steps may still undo what it did — the lock is still its
/// own, and the holder waits for the undo too (it carries `WRITE_ENV`).
const HELD_ENV: &str = "VRCAST_LIMITS_HELD";

/// The mark every step a change runs under the lock carries in its environment, from the
/// moment it starts on the server (T628, the same idea as `VRCAST_DEPLOY_RUN` in T609):
/// `<id>.f` for a step going forward, `<id>.u` for one putting back (`Txn::forward`,
/// `Txn::back`).
///
/// The holder waits for every live process whose mark starts with its own change's `<id>`
/// before it lets the lock go: a step already sent goes on running on the server whatever
/// happens to the client — a laptop asleep, a client frozen, a link gone — and letting the
/// next change in while it runs is what mixed two changes together (QA-20 №4).
pub const WRITE_ENV: &str = "VRCAST_LIMITS_WRITE";

/// How long the holder waits for the steps of its change, once nobody is heard from,
/// before it stops them (T628).
///
/// A step is one command, and the client itself gives one up after `EXEC_CEILING`; a step
/// still running that long after the door closed is taken for hung and stopped — TERM,
/// then KILL (`domain::marked::stop_script`, as T609 stops a deployment's command) — and,
/// for as long as the stop says it is still alive, TERM and KILL again (T634): the lock is
/// never let go under a step of its change that has not been seen to end.
pub const DRAIN_CEILING: Duration = crate::ssh::exec::EXEC_CEILING;

#[derive(Debug, thiserror::Error)]
pub enum LimitError {
    /// The web server refused the configuration. Nothing was changed for anybody.
    #[error("the serving refused the new configuration: {0}")]
    ValidateFailed(String),

    /// The configuration was sound and the reload still failed.
    #[error("the serving would not take the new configuration: {0}")]
    ReloadFailed(String),

    /// The change went in and the serving stopped answering. What was there before is back.
    #[error("the serving stopped answering, so the previous configuration was put back")]
    ServingStopped,

    /// The worst case: the change failed **and** putting the old one back failed too.
    ///
    /// Told apart from the rest on purpose. Everything else leaves a working server and a
    /// person who can try again; this one needs them to go and look.
    #[error("the serving is broken and the previous configuration would not go back: {0}")]
    RollbackFailed(String),

    /// Another change reached the server between this call's read and its write (T600).
    ///
    /// The same idea as `ManifestIoError::Conflict`, adapted to a text config file instead
    /// of JSON: nothing was written, the file on the server is exactly as the OTHER change
    /// left it, and the caller is expected to read again and either retry or give up —
    /// never to treat this as a fault of the serving itself.
    #[error("the rules were changed by another change in between: read generation {base}, server has {current}")]
    Conflict { base: u64, current: u64 },

    /// Another change on this server held the lock for longer than `LOCK_WAIT` (T603).
    ///
    /// Nothing was touched. For the caller it means the same as `LimitError::Conflict` —
    /// somebody else is changing the rules; read again and retry — and it is reported under
    /// the same code.
    #[error("another change of the rules is in progress on this server")]
    Busy,

    /// A step of putting the new rules in place failed (T603): a copy, a move, the lock
    /// lost. Everything this change had touched was put back; nothing is in force that was
    /// not before.
    #[error("the new rules could not be put in place, and nothing was changed: {0}")]
    WriteFailed(String),

    /// The medium the change is about has no quality set to shorten (T215, T602): read
    /// under the lock, just before anything would be written. Nothing was touched.
    #[error("the medium {0} has no quality set to shorten")]
    NoLadder(String),

    #[error(transparent)]
    Ssh(#[from] SshError),
}

/// The serving, as far as limits are concerned.
pub struct Serving<'a> {
    pub conn: &'a Connection,
    /// Where the media are, as the person's own profile says.
    pub video_dir: &'a str,
    /// The file this application owns, e.g. `/etc/caddy/vrcast-limits.conf`.
    pub conf_path: &'a str,
    /// The main configuration, only ever read: `/etc/caddy/Caddyfile`.
    pub main_conf: &'a str,
    /// Where the media sit in an address, e.g. `/videos`.
    pub serving_prefix: &'a str,
    /// Something a viewer would ask for, to prove the serving still answers.
    pub check_url: &'a str,
    /// `user:group` the files must belong to.
    pub owner: &'a str,
}

/// One shortened description one change writes (T603, T602).
///
/// **A list of paths, not one file per medium** (T603): keeping what was there and putting
/// it back works over whatever paths this list names. Since T602 the list holds one entry
/// per distinct (medium, ceiling) of the rules after the change, and only writes — what is
/// no longer needed is removed after the change has been proven (`Serving::sweep`), never
/// inside the part that may have to be undone.
struct Description {
    path: String,
    text: String,
}

/// Everything one transaction needs to name on the server.
struct Txn<'a> {
    /// Unique to this change: names its staged files, marks its lock holder and every
    /// writing step it sends (T628).
    id: String,
    /// The processes on the server that say the lock is this change's.
    holder: Holder,
    base: u64,
    changes: &'a [Description],
    /// The directories the descriptions go into, each once, every parent before its
    /// children (`_slow/<slug>` before `_slow/<slug>/<cap>`). Those this change had to make
    /// are removed again if it is undone.
    dirs: &'a [String],
    /// Whether a step of this change going forward was sent and its end was never heard —
    /// given up on at its ceiling, its channel failed, or it closed with no exit status
    /// (T635). Such a step may still be running on the server; nothing is put back before
    /// the barrier has seen it end (`Serving::settle`).
    unheard: std::sync::atomic::AtomicBool,
}

impl Txn<'_> {
    fn new_generation(&self) -> u64 {
        self.base + 1
    }
    fn staged(&self, path: &str) -> String {
        format!("{path}.{}.new.tmp", self.id)
    }
    fn kept(&self, path: &str) -> String {
        format!("{path}.{}.was.tmp", self.id)
    }
    /// The mark of a step going forward (T628).
    fn forward(&self) -> String {
        format!("{}.f", self.id)
    }
    /// The mark of a step putting back (T628).
    fn back(&self) -> String {
        format!("{}.u", self.id)
    }
}

/// What the preparing step found, needed to put things back.
struct Prepared {
    /// Whether the rules file existed before this change.
    had_conf: bool,
    /// For each shortened change, in order: whether the file existed before.
    present: Vec<bool>,
    /// For each of `Txn::dirs`, in order: whether this change made it.
    made: Vec<bool>,
}

/// The lock that makes a change one transaction (T603), held for as long as this lives.
///
/// Held by a process on the server — `flock` running a shell whose door waits on our
/// channel — rather than by a lock file we create and remove: the door closes the moment the
/// channel closes, so a client that crashed or a connection that dropped never leaves the
/// lock behind for longer than its own steps still running on the server take (T628).
/// Since T618 the channel itself belongs to a task that renews the lock every
/// `RENEW_EVERY` (`renew_until`) until it is told to give the lock back. Dropping this
/// without `TxnLock::release` drops that task's stop signal, and the task closes the
/// channel at once (russh's `ChannelStream` closes it on drop), which frees the lock the
/// same way.
struct TxnLock {
    holder: Holder,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    renewing: tokio::task::JoinHandle<Renewal>,
}

impl TxnLock {
    /// Give the lock back, and wait until the server really has.
    ///
    /// End-of-input closes the door, the holder waits for this change's steps (none are
    /// left by now, unless one was given up on at `EXEC_CEILING`), ends, `flock` with it,
    /// and only then does the server close its side of the channel — so the end of the
    /// channel means the lock is free, not that it is about to be.
    async fn release(mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        match (&mut self.renewing).await {
            Ok(Renewal {
                given_back: true, ..
            }) => {}
            outcome => tracing::warn!(
                ?outcome,
                "the server did not confirm the limits lock was given back; closing the channel"
            ),
        }
    }
}

/// The two processes of a lock's holder that the steps of the change look at (T628; see
/// `holder_command`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Holder {
    /// The door: alive, carrying `TXN_ENV=<id>`, while the client is heard from. A step
    /// going forward starts only while it is (`guard`).
    pub door: u32,
    /// The holder itself: carrying `HELD_ENV=<id>` from the moment the lock is taken until
    /// it has waited once for every step of the change going forward. Putting back starts
    /// only while it does (`guard_held`).
    pub held: u32,
}

/// Read the holder's first line: `LOCKED <door> <holder>` (T628).
///
/// `None` for anything else — including the T618 holder's `LOCKED <pid>`, which cannot be
/// what this application's own holder says.
pub fn read_locked(line: &str) -> Option<Holder> {
    let mut words = line.trim().strip_prefix("LOCKED ")?.split_whitespace();
    let door = words.next()?.parse().ok()?;
    let held = words.next()?.parse().ok()?;
    if words.next().is_some() || door == 0 || held == 0 {
        return None;
    }
    Some(Holder { door, held })
}

/// What renewing a lock came to, once it stopped (T618).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Renewal {
    /// How many renewals were written.
    pub sent: u32,
    /// Whether one could not be written: the channel is gone, and with it — by the time
    /// the server notices — the lock. Every later step finds that out for itself (`guard`).
    pub failed: bool,
    /// Whether the lock was given back and the server confirmed it by closing the channel.
    pub given_back: bool,
}

/// Keep a lock's holder told that we are alive, until told to stop (T618).
///
/// Writes one newline every `every` to `stream` — the holder's input — which is what the
/// holder waits on (`holder_command`): a line within `LOCK_SILENCE` and it keeps the lock,
/// none and it lets go. Runs on its own, beside whatever step of the change is under way,
/// so a step that takes minutes (a slow `caddy validate`, the rollback after it) is covered
/// for its whole length.
///
/// `stop` sent: gives the lock back — end of input — and waits up to `RELEASE_WAIT` for the
/// server to close the channel, i.e. for the lock to be really free. `stop` dropped (the
/// change was abandoned without `release`): the stream is dropped at once, which closes the
/// channel and frees the lock the same way.
///
/// `pub` for the unit test, which drives it over an in-memory pipe.
pub async fn renew_until<S>(
    mut stream: S,
    every: Duration,
    mut stop: tokio::sync::oneshot::Receiver<()>,
) -> Renewal
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let mut renewal = Renewal {
        sent: 0,
        failed: false,
        given_back: false,
    };
    let mut tick = tokio::time::interval_at(tokio::time::Instant::now() + every, every);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            biased;
            asked = &mut stop => {
                if asked.is_ok() {
                    let _ = stream.shutdown().await;
                    let mut rest = Vec::new();
                    let closed =
                        tokio::time::timeout(RELEASE_WAIT, stream.read_to_end(&mut rest))
                            .await
                            .is_ok_and(|r| r.is_ok());
                    // A lock whose channel had already failed was lost, not given back.
                    renewal.given_back = closed && !renewal.failed;
                }
                return renewal;
            }
            _ = tick.tick(), if !renewal.failed => {
                // Bounded, so a write stuck on a channel that is not taking anything does
                // not keep the stop from being heard. A write that is only slow is tried
                // again at the next tick — the link may come back within `LOCK_SILENCE`,
                // and until the server has gone that long without a line the lock is still
                // ours. Only a write the channel refuses outright (it is closed) ends the
                // renewing: nothing written after that could reach the holder.
                let wrote = tokio::time::timeout(every, async {
                    stream.write_all(b"\n").await?;
                    stream.flush().await
                })
                .await;
                match wrote {
                    Ok(Ok(())) => renewal.sent += 1,
                    Ok(Err(e)) => {
                        renewal.failed = true;
                        tracing::warn!(
                            error = %e,
                            "the limits lock could not be renewed: its channel is closed, so \
                             the lock is gone and the next step of the change will stop"
                        );
                    }
                    Err(_) => tracing::warn!(
                        "renewing the limits lock is slow; trying again at the next tick"
                    ),
                }
            }
        }
    }
}

/// The command that takes the lock and holds it (T603, T618, T628).
///
/// `flock` runs a `bash` — the **holder**, carrying `HELD_ENV=<id>` — and the lock is free
/// the moment that process ends. The holder, in this order:
///
///  1. ignores HUP and PIPE: a client that vanished must not take the holder with it while
///     a step of its change is still running;
///  2. runs the **door** (`TXN_ENV=<id>`) in the foreground: it says `LOCKED <door> <holder>`
///     and reads its input a line at a time, waiting at most `LOCK_SILENCE` for each —
///     every renewal restarts the wait; end of input (ours on release, or the server's when
///     the connection dies) or a wait that runs out closes the door (T618);
///  3. **waits for every live step of this change going forward** — processes carrying
///     `WRITE_ENV=<id>.f`, still running on the server — for up to `DRAIN_CEILING`, then
///     stops whatever is left, TERM and then KILL: `domain::marked::stop_script`, the stop
///     of T609 (T628) — and **reads its answer** (`DRAIN`, T634): as long as it says a step
///     is still alive, TERM and KILL again, with no limit on how many times; only a
///     confirmed end, or `/proc` that cannot be read at all, lets it go on;
///  4. replaces itself (`exec`, the same PID — `flock` is still waiting on it) by the same
///     wait **without** `HELD_ENV`, for every step of the change (`WRITE_ENV=<id>…`): a
///     putting back that got in before the mark went is waited for — with a `DRAIN_CEILING`
///     of its own, counted from here, and the same reading of the answer — and none can get
///     in after;
///  5. ends — and with it `flock`, and the lock.
///
/// **Why nothing slips past.** Every step carries its mark from its first instruction, and
/// checks the door or the holder's mark only after that (`write_step`, `guard`,
/// `guard_held`). A step going forward that found the door open existed before the door
/// closed, and 3 looks for it after; one putting back that found `HELD_ENV` existed before
/// 4 dropped it, and 4 looks for it after. A step that found them gone does nothing.
///
/// Nothing but `flock` holds the lock's descriptor past the holder: every process the
/// holder starts ends before it does. The holder's output after the door goes nowhere —
/// the channel may be gone by then. `bash` by name, not `sh`: `read -t` is not in every
/// `sh` (Ubuntu's `dash` has none), and the stop script needs `mapfile -d` (bash 4.4).
///
/// `pub` so the Docker test can run the very same holder with an input that stays open and
/// says nothing — a client that went silent without its channel closing.
pub fn holder_command(id: &str, lock_path: &str) -> String {
    holder_command_draining(id, lock_path, DRAIN_CEILING)
}

/// The same holder, with the wait for the change's steps before the first TERM given rather
/// than `DRAIN_CEILING` — so the Docker test of a step that will not die (T634) need not
/// sit through ten minutes of a step that is merely slow first. Nothing in the application
/// passes anything but `DRAIN_CEILING`.
pub fn holder_command_draining(id: &str, lock_path: &str, drain: Duration) -> String {
    let door = format!(
        "echo \"LOCKED $$ $1\"; while IFS= read -r -t {silence} _; do :; done",
        silence = LOCK_SILENCE.as_secs(),
    );
    let wait_args = format!(
        "{term} {kill} {grace}",
        term = super::marked::TERM_TICKS,
        kill = super::marked::KILL_TICKS,
        grace = drain.as_secs(),
    );
    let holder = format!(
        "trap '' HUP PIPE\n\
         id=$1\n\
         stop={stop}\n\
         drain={drain}\n\
         {TXN_ENV}=\"$id\" bash -c {door} vrcast-limits-door \"$$\"\n\
         exec </dev/null >/dev/null 2>&1\n\
         bash -c \"$drain\" vrcast-limits-wait \"$stop\" \"$id.f\" {wait_args}\n\
         exec env -u {HELD_ENV} bash -c \"$drain\" vrcast-limits-last \"$stop\" \"$id\" {wait_args}\n",
        stop = super::shell_quote(&crate::domain::marked::stop_script(WRITE_ENV)),
        drain = super::shell_quote(&drain_script()),
        door = super::shell_quote(&door),
    );
    let id = super::shell_quote(id);
    format!(
        "flock -x -w {wait} -E 75 {lock} env {HELD_ENV}={id} bash -c {holder} \
         vrcast-limits-holder {id} || echo \"NOT_LOCKED $?\"",
        wait = LOCK_WAIT.as_secs(),
        lock = super::shell_quote(lock_path),
        holder = super::shell_quote(&holder),
    )
}

/// The holder's wait for the steps of its change, and the barrier before a putting back
/// (T634, T635): the stop of T609 (`domain::marked::stop_script`, passed as `$1`) run **again
/// and again** until its answer is one that lets go.
///
/// Arguments: the stop script, the mark (a prefix), the TERM and KILL waits in 100 ms
/// ticks, how many seconds the first round waits for the steps to end on their own, and —
/// optional — how many rounds at most (none, or 0: no limit).
///
/// **Reading the answer is the point** (QA-21 №1). Before T634 the answer went to
/// `/dev/null`, and `VRCAST_STOP alive …` — TERM and KILL both sent, and a step of the
/// change still there — let the lock go exactly like a confirmed end: the limit on the wait
/// was a limit on the lock. Now only these let go:
///
///  * `none`, `ended`, `term`, `kill` — nothing of the change is alive: confirmed;
///  * `unreadable` — `/proc` cannot be read here (bash < 4.4, a `/proc` hidden from us):
///    nothing can ever be confirmed on this server, and holding the lock for good would
///    only make every change of the rules on it fail. The owner's decision (2026-09-30):
///    let go, as before T628.
///
/// Anything else — `alive`, and an answer that is not one at all — is "not confirmed": the
/// next round sends TERM, then KILL again (no waiting first this time), a second after the
/// last. With no limit on the rounds (the holder) this goes on for as long as a step of the
/// change is alive, and the lock with it; every other change meanwhile waits `LOCK_WAIT`
/// and is turned away (`LIMITS_CONFLICT`). With a limit (the barrier) the last answer is
/// said as `VRCAST_DRAIN unconfirmed …`.
///
/// Prints the answer that ended it — nobody reads it in the holder, whose output goes
/// nowhere; the barrier reads it (`read_settled`).
const DRAIN: &str = r#"set -u
STOP="$1"; WANT="$2"; TERM_TICKS="$3"; KILL_TICKS="$4"; GRACE_S="$5"; ROUNDS="${6:-0}"
round=0
while :; do
  said=$(VRCAST_HLS_SELFCHECK=1 bash -c "$STOP" vrcast-limits-stop "$WANT" "$TERM_TICKS" "$KILL_TICKS" "$GRACE_S" signal)
  round=$((round + 1))
  case "$said" in
    "VRCAST_STOP none"|"VRCAST_STOP ended "*|"VRCAST_STOP term "*|"VRCAST_STOP kill "*|"VRCAST_STOP unreadable")
      echo "$said"; exit 0 ;;
    "VRCAST_STOP "*) ;;
    # No answer at all: the stop could not even start its scan. Where that is because
    # `/proc` is not there, it never will — the same as `unreadable`.
    *) [ -r "/proc/$$/stat" ] || { echo "VRCAST_STOP unreadable"; exit 0; } ;;
  esac
  if [ "$ROUNDS" -gt 0 ] && [ "$round" -ge "$ROUNDS" ]; then
    echo "VRCAST_DRAIN unconfirmed after $round rounds: ${said//$'\n'/ }"; exit 0
  fi
  GRACE_S=0
  sleep 1
done
"#;

/// The drain script (`DRAIN`), `pub` so the unit test can run it against a stop that
/// answers what the test tells it to.
pub fn drain_script() -> String {
    DRAIN.to_owned()
}

/// How many rounds of TERM and KILL the barrier before a putting back tries (T635) before
/// it gives up and the putting back is refused. Each round is at most 5 s + 5 s + a second,
/// so three are about half a minute past whatever the step still had of its own time.
pub const BARRIER_ROUNDS: u32 = 3;

/// The barrier's command (T635): wait for every live step of this change going forward
/// (`<id>.f`) — `grace_s` for it to end on its own, then TERM and KILL for up to
/// `BARRIER_ROUNDS` rounds — and say how it went (`read_settled`).
///
/// Not marked itself: it is not a step of the change, and the holder must not wait for it.
pub fn barrier_command(forward_mark: &str, grace_s: u64) -> String {
    format!(
        "bash -c {drain} vrcast-limits-barrier {stop} {mark} {term} {kill} {grace_s} {BARRIER_ROUNDS}",
        drain = super::shell_quote(DRAIN),
        stop = super::shell_quote(&crate::domain::marked::stop_script(WRITE_ENV)),
        mark = super::shell_quote(forward_mark),
        term = super::marked::TERM_TICKS,
        kill = super::marked::KILL_TICKS,
    )
}

/// How long the barrier gives an unheard step to end on its own before TERM (T635).
///
/// Every step of a change is a few file operations or one `caddy validate`/`reload` — 17–18
/// ms each on the test server (2026-09-23). A step whose answer was lost on a channel that
/// broke is, as a rule, long over; ten seconds is room for a slow disk. One given up on at
/// its ceiling has had ten minutes already and is taken for hung.
pub const BARRIER_GRACE: Duration = Duration::from_secs(10);

/// What the barrier found (T635).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Settled {
    /// No step of the change going forward is alive: putting back may begin.
    Confirmed,
    /// `/proc` cannot be read on this server, so nothing can be confirmed there — and the
    /// holder lets the lock go on the same answer (T634). Putting back goes ahead as it did
    /// before T635; logged.
    Unreadable,
    /// A step may still be running, or the answer was no answer: putting back must not
    /// begin.
    NotConfirmed(String),
}

/// Read the barrier's answer. Only the stop's own words for "gone" confirm, and only its
/// exact `unreadable` counts as that; anything else is not a confirmation.
pub fn read_settled(text: &str) -> Settled {
    use crate::domain::marked::StopReport;
    if text.lines().any(|l| l.trim() == "VRCAST_STOP unreadable") {
        return Settled::Unreadable;
    }
    match crate::domain::marked::read_stop(text) {
        StopReport::Confirmed { .. } => Settled::Confirmed,
        StopReport::StillAlive(who) => Settled::NotConfirmed(format!("still alive: {who}")),
        StopReport::StillRunning(who) => Settled::NotConfirmed(format!("still running: {who}")),
        StopReport::Unreadable(said) => Settled::NotConfirmed(said),
    }
}

/// Whether a step's outcome was **heard** (T635): the server said the step's shell ended,
/// with a status. A step given up on at its ceiling, one whose channel failed, and one
/// whose channel closed with no exit status were not — the step may still be running on the
/// server, and a putting back must wait for it (`settle_then`).
pub fn unheard(outcome: &Result<crate::ssh::CommandOutput, SshError>) -> bool {
    !matches!(outcome, Ok(out) if out.exit_code.is_some())
}

/// Put back only once the barrier has confirmed nothing of the change is still going
/// forward (T635): `settle` first, and `undo` only if it succeeded. Its error is the
/// putting back's, and the putting back never started.
///
/// `pub` so the unit test can check the order with steps that only record themselves.
pub async fn settle_then<S, SF, U, UF>(settle: S, undo: U) -> Result<(), String>
where
    S: FnOnce() -> SF,
    SF: std::future::Future<Output = Result<(), String>>,
    U: FnOnce() -> UF,
    UF: std::future::Future<Output = Result<(), String>>,
{
    settle()
        .await
        .map_err(|e| format!("putting back was not started: {e}"))?;
    undo().await
}

tokio::task_local! {
    /// The ceiling on one step of a change, where a test sets one (`with_step_ceiling`).
    static STEP_CEILING: Duration;
}

/// Run `f` — a change of the rules — with its steps given up on after `ceiling` rather than
/// `EXEC_CEILING` (T635), so the Docker test of a step that outlives its client's wait need
/// not wait ten minutes. Only for the task `f` runs in; nothing in the application calls it.
pub async fn with_step_ceiling<F: std::future::Future>(ceiling: Duration, f: F) -> F::Output {
    STEP_CEILING.scope(ceiling, f).await
}

fn step_ceiling() -> Duration {
    STEP_CEILING
        .try_with(|c| *c)
        .unwrap_or(crate::ssh::exec::EXEC_CEILING)
}

/// A step of change `id`, as it is sent (T628): carrying `WRITE_ENV=<id>` from its first
/// instruction, so the lock's holder waits for it — and everything it starts — before it
/// lets the lock go.
///
/// Run by the login shell (`$SHELL`), as the step was before T628, so no step's text
/// changes meaning; its answer and its exit status are the step's own.
///
/// `pub` so the Docker test can send a slow step of a change exactly as the change does.
pub fn write_step(id: &str, script: &str) -> String {
    format!(
        "{WRITE_ENV}={} \"${{SHELL:-/bin/sh}}\" -c {}",
        super::shell_quote(id),
        super::shell_quote(script)
    )
}

impl Serving<'_> {
    /// What limits the **server** says are in force, and the generation they were read at
    /// (T600).
    ///
    /// Read from the server rather than from a note kept here (FR-064): a note goes stale
    /// the moment somebody edits the server by hand, and a list that does not match the
    /// server is worse than no list. The generation comes from the very same read as the
    /// list — a caller that means to write back must pass exactly this number as
    /// `apply()`'s `base_generation`, or the check below is comparing against a read that
    /// never happened.
    ///
    /// Not under the lock: a reader may see a change that is later rolled back. That is
    /// why a rollback never hands an old generation number out again (see `undo`).
    pub async fn limits(&self) -> Result<(Vec<Limit>, u64), LimitError> {
        let out = self
            .conn
            .exec(&format!(
                "cat {} 2>/dev/null || true",
                super::shell_quote(self.conf_path)
            ))
            .await?;
        Ok((
            limits_conf::parse(&out.stdout),
            limits_conf::read_generation(&out.stdout),
        ))
    }

    /// Put a set of limits in force, whole.
    ///
    /// **All of it is one transaction** (T603): from before the first file is touched until
    /// after the last check or the rollback, this change holds a lock on the server that
    /// every other `apply()` of the same rules waits for. Between this change putting its
    /// files in place and deciding whether they stay, nobody else changes either the rules
    /// or the shortened descriptions. Under that lock, in this order:
    ///
    ///  0. the rules in force are read, and the change stops with `Conflict` before
    ///     touching anything unless they are at `base_generation`; the quality set of every
    ///     medium the new rules name is read, and one shortened description is made for
    ///     each distinct (medium, ceiling) among them (T602, `Serving::descriptions`);
    ///  1. the generation is checked again (T600); the directories the descriptions go into
    ///     are made if they are missing, and the descriptions this change will replace are
    ///     kept aside;
    ///  2. the new files are staged beside where they will go, not yet in force;
    ///  3. the rules file is kept as `.previous`, the shortened descriptions go in — before
    ///     the rules, since a rule pointing at a description that is not there yet would
    ///     serve a limited viewer nothing — and then the rules. Every copy and every move
    ///     is checked; a failed copy means nothing is replaced, a failed move means an
    ///     error, never success;
    ///  4. the web server is asked to check what is now in place **by its own means** — our
    ///     opinion of a configuration file is worth nothing;
    ///  5. it is reloaded;
    ///  6. the serving is asked for something a viewer would ask for, over the address a
    ///     viewer uses — from here, not from the server (see `Serving::serving_answers`);
    ///  7. only now, still under the lock, what the new rules no longer reach is removed
    ///     (`Serving::sweep`): the descriptions of ceilings no rule names any more, every
    ///     pre-T602 `_slow/<slug>/master.m3u8` of a medium named before or after, and the
    ///     directories left empty.
    ///
    /// Any failure from step 1 on and before step 7 puts everything this change touched
    /// back (`undo`): the rules as they were, under a **new** generation number, the
    /// shortened descriptions as they were, or absent — with any directory this change made
    /// for them — if they were absent. Nothing has been removed by then, so there is
    /// nothing removed to bring back. After any outcome the lock is free again.
    ///
    /// **Nothing is put back while a step going forward may still be running** (T635). A
    /// step whose end was not heard — given up on at its ceiling, its channel failed — may go
    /// on on the server; the putting back waits for it first (`Serving::settle`), and if its
    /// end cannot be confirmed it is not started at all: `RollbackFailed`, and the lock stays
    /// with the holder until the step is gone (T634).
    ///
    /// **Why removing comes last (T602).** Until the reload the rules the web server holds
    /// are the old ones, and they point at the old files; removing one of those before the
    /// new rules are in force and proven would give a limited viewer nothing for as long as
    /// that takes, and for good if the change is then rolled back. A removal that fails at
    /// step 7 does not turn the change into a failure: the new rules are in force and
    /// checked, and a file nothing reaches is untidy rather than wrong.
    ///
    /// **Why the checking happens after the file is in place and not before.** The main
    /// configuration imports this file by name; until the new content is under that name
    /// there is nothing for the web server to check. That is safe because of something the
    /// web server does rather than something we do: a reload that is refused leaves the
    /// **previous** configuration running. So a bad file is caught while the old one is
    /// still serving.
    ///
    /// **Why a lock held by a process, not the T600 script's own `flock`.** A lock taken and
    /// released inside one `exec()` covers one `exec()`; the checking needs several, and the
    /// viewer's check is not on the server at all. The lock here is held by a process whose
    /// life is tied to a channel of its own (`Serving::lock`), for as long as the whole
    /// change takes. The steps under it do **not** take the lock again: `flock` on a second
    /// descriptor of the same file would wait on our own holder, forever.
    ///
    /// **`limits`** is the whole list to be in force afterwards — putting a limit on and
    /// taking one off are the same thing here (`commands/limits.rs::api::limit_set`/
    /// `limit_clear`). **`needs_ladder`** names the medium the person is changing: if it has
    /// no quality set when the change reads it, the change stops with `NoLadder` having
    /// touched nothing. Any other medium without one keeps whatever description it already
    /// has (see `Serving::descriptions`).
    ///
    /// **`base_generation`** must be the generation `Serving::limits()` returned alongside
    /// the list `limits` was made from. A mismatch returns `LimitError::Conflict` having
    /// written nothing. A match writes `generation = base_generation + 1`.
    pub async fn apply(
        &self,
        limits: &[Limit],
        needs_ladder: Option<&str>,
        base_generation: u64,
    ) -> Result<(), LimitError> {
        let id = uuid::Uuid::new_v4().simple().to_string();
        let lock = self.lock(&id).await?;
        let outcome = self
            .locked(&id, lock.holder, limits, needs_ladder, base_generation)
            .await;
        lock.release().await;
        outcome
    }

    /// Step 0 and everything after it, the lock already held.
    async fn locked(
        &self,
        id: &str,
        holder: Holder,
        limits: &[Limit],
        needs_ladder: Option<&str>,
        base_generation: u64,
    ) -> Result<(), LimitError> {
        // What is in force now, read under the lock: this is what the new rules replace,
        // and so what decides what will no longer be reached.
        let (before, current) = self.limits().await?;
        if current != base_generation {
            return Err(LimitError::Conflict {
                base: base_generation,
                current,
            });
        }
        let plan = slow_master::plan(self.video_dir, &before, limits);
        let changes = self.descriptions(&plan, needs_ladder).await?;
        let dirs = dirs_for(self.video_dir, &changes);

        let txn = Txn {
            id: id.to_owned(),
            holder,
            base: base_generation,
            changes: &changes,
            dirs: &dirs,
            unheard: std::sync::atomic::AtomicBool::new(false),
        };
        self.under_lock(&txn, limits).await?;
        self.sweep(&txn, &plan).await;
        Ok(())
    }

    /// One shortened description for each (medium, ceiling) the new rules name (T602),
    /// each made from the medium's quality set as it is **now** — so a set that was rebuilt
    /// since the limit was put on is what the limited viewer is shown.
    ///
    /// **A medium the change is not about that has no quality set any more** (it was
    /// removed, or rebuilt as a single file) does not stop the change: the person is
    /// changing a different medium's limit, and nothing they could do here would give that
    /// one a set back. Nothing is written for it — whatever description of it is already
    /// there stays as it is, and a rule that had none still leads nowhere, exactly as it
    /// did before this change. Every unlimited viewer of such a medium is in the same place
    /// (the address the rule matches serves nothing any more either).
    async fn descriptions(
        &self,
        plan: &slow_master::SlowPlan,
        needs_ladder: Option<&str>,
    ) -> Result<Vec<Description>, LimitError> {
        let mut ladders: std::collections::BTreeMap<&str, Vec<crate::domain::hls_master::Variant>> =
            std::collections::BTreeMap::new();
        for (slug, _) in &plan.descriptions {
            if ladders.contains_key(slug.as_str()) {
                continue;
            }
            let master = format!(
                "{}/{slug}/master.m3u8",
                self.video_dir.trim_end_matches('/')
            );
            let out = self
                .conn
                .exec(&format!(
                    "cat {} 2>/dev/null || true",
                    super::shell_quote(&master)
                ))
                .await?;
            let variants = crate::domain::hls_master::parse(&out.stdout).unwrap_or_default();
            if variants.is_empty() {
                tracing::warn!(
                    slug = %slug,
                    "a limited medium has no quality set any more; its description is left as it is"
                );
            }
            ladders.insert(slug.as_str(), variants);
        }
        if let Some(slug) = needs_ladder {
            if !matches!(ladders.get(slug), Some(v) if !v.is_empty()) {
                return Err(LimitError::NoLadder(slug.to_owned()));
            }
        }

        Ok(plan
            .descriptions
            .iter()
            .filter_map(|(slug, cap)| {
                let variants = ladders.get(slug.as_str()).filter(|v| !v.is_empty())?;
                Some(Description {
                    path: slow_master_path(self.video_dir, slug, *cap),
                    text: shorten(variants, *cap, self.serving_prefix, slug).text,
                })
            })
            .collect())
    }

    /// Take the lock, waiting up to `LOCK_WAIT` for whoever holds it.
    ///
    /// The holder (`holder_command`) takes the lock and opens its **door**, which waits on
    /// this channel for a sign of life: the door closes on end-of-input — ours, or the
    /// server's when the connection dies — or after `LOCK_SILENCE` with nothing heard
    /// (T618). The lock itself goes only after that, once every step of this change still
    /// running on the server has ended (T628). From the moment the lock is ours a task of
    /// its own renews it every `RENEW_EVERY` (`renew_until`), until `TxnLock::release`. The
    /// holder names the door's PID and its own (`read_locked`) so later steps can check
    /// they are still there and still ours.
    ///
    /// The channel takes an ordinary place, not a standing one: it lasts as long as one
    /// change, and the watching of viewers keeps both of its places (T153). A change uses
    /// at most two places at once — this one and the step running under it. The place goes
    /// with the renewing task and is given back when it ends.
    async fn lock(&self, id: &str) -> Result<TxnLock, LimitError> {
        let permit = self.conn.acquire_channel().await?;
        let channel = self.conn.open_session().await?;
        channel
            .exec(true, holder_command(id, &self.lock_path()))
            .await
            .map_err(SshError::protocol)?;
        let mut stream = channel.into_stream();

        let first =
            tokio::time::timeout(LOCK_WAIT + Duration::from_secs(30), first_line(&mut stream))
                .await;
        let line = match first {
            Ok(Ok(line)) => line,
            Ok(Err(e)) => {
                return Err(LimitError::Ssh(SshError::Exec(format!(
                    "the limits lock could not be taken: {e}"
                ))))
            }
            // The server's own wait is shorter than ours; getting here means it did not say
            // anything at all. Dropping the stream closes the channel, and a lock taken
            // after that is given straight back (the holder reads a closed input).
            Err(_) => return Err(LimitError::Busy),
        };

        if let Some(holder) = read_locked(&line) {
            let (stop, stopped) = tokio::sync::oneshot::channel();
            let renewing = tokio::spawn(async move {
                let _permit = permit;
                renew_until(stream, RENEW_EVERY, stopped).await
            });
            return Ok(TxnLock {
                holder,
                stop: Some(stop),
                renewing,
            });
        }
        if line.trim() == "NOT_LOCKED 75" {
            return Err(LimitError::Busy);
        }
        Err(LimitError::Ssh(SshError::Exec(format!(
            "the limits lock could not be taken: {:?}",
            line.trim()
        ))))
    }

    fn lock_path(&self) -> String {
        format!("{}.lock", self.conf_path)
    }

    fn previous_path(&self) -> String {
        format!("{}.previous", self.conf_path)
    }

    /// The change itself, the lock already held.
    async fn under_lock(&self, txn: &Txn<'_>, limits: &[Limit]) -> Result<(), LimitError> {
        let prepared = match self.prepare(txn).await {
            Ok(prepared) => prepared,
            Err(e) => {
                // The script removes its own copies when it fails; this is for when it
                // never got to say so (a dropped channel, a reply that made no sense).
                self.discard_kept(txn).await;
                return Err(e);
            }
        };

        // From here on, anything that fails puts back what this change touched.
        let text = limits_conf::build(limits, self.serving_prefix, txn.new_generation());
        let staged = self.stage(txn, &text).await;
        let swapped = match staged {
            Ok(()) => self.swap(txn).await,
            Err(e) => Err(e),
        };
        if let Err(LimitError::Conflict { base, current }) = swapped {
            // Only possible if somebody edited the file by hand without the lock: nothing
            // of ours was moved (the check is the first line of the step), so there is
            // nothing to put back and nothing that is ours to put it over — apart from the
            // directories this change made for its descriptions, which go again.
            self.discard_staged(txn).await;
            self.discard_kept(txn).await;
            self.unmake_dirs(txn, &prepared).await;
            return Err(LimitError::Conflict { base, current });
        }
        if let Err(e) = swapped {
            return Err(match self.put_back(txn, &prepared).await {
                Ok(()) => e,
                Err(undo) => LimitError::RollbackFailed(format!("{e}; then: {undo}")),
            });
        }

        if let Err(e) = self.check_and_reload(txn).await {
            self.roll_back(txn, &prepared).await?;
            return Err(e);
        }
        if !self.serving_answers().await {
            self.roll_back(txn, &prepared).await?;
            return Err(LimitError::ServingStopped);
        }

        self.finish(txn).await;
        Ok(())
    }

    /// Step 1: check the generation, make room for the shortened descriptions, and keep
    /// aside the ones this change will replace.
    ///
    /// On any failure this step removes what it had kept aside and the directories it had
    /// made itself, and nothing else has been touched yet — there is nothing for `undo` to
    /// do.
    async fn prepare(&self, txn: &Txn<'_>) -> Result<Prepared, LimitError> {
        let mut script = self.script_head(txn);
        let kept: Vec<String> = txn
            .changes
            .iter()
            .map(|c| super::shell_quote(&txn.kept(&c.path)))
            .collect();
        // The directories this step made go again on its own failure, deepest first.
        let mut unmake = String::new();
        for (i, dir) in txn.dirs.iter().enumerate().rev() {
            unmake.push_str(&format!(
                " [ \"$m{i}\" = 1 ] && rmdir {} 2>/dev/null;",
                super::shell_quote(dir)
            ));
        }
        for i in 0..txn.dirs.len() {
            script.push_str(&format!("m{i}=0\n"));
        }
        script.push_str(&format!(
            "discard() {{ rm -f {};{unmake} true; }}\n",
            kept.join(" ")
        ));
        // Parents first: a directory counts as made by this change only if it was not
        // there a moment ago, so the ceiling's is asked about after the medium's is made.
        for (i, dir) in txn.dirs.iter().enumerate() {
            let dir = super::shell_quote(dir);
            script.push_str(&format!(
                "if [ -d {dir} ]; then echo 'DIR {i} OLD'; else\n\
                 mkdir -p {dir} || {{ discard; echo FAIL_MAKE_ROOM; exit 5; }}\n\
                 m{i}=1; echo 'DIR {i} MADE'\n\
                 chown {owner} {dir} 2>/dev/null || true\n\
                 fi\n",
                owner = super::shell_quote(self.owner),
            ));
        }
        for (i, change) in txn.changes.iter().enumerate() {
            let path = super::shell_quote(&change.path);
            script.push_str(&format!(
                "if [ -e {path} ]; then\n\
                 cp -p {path} {kept} || {{ discard; echo FAIL_KEEP_SHORTENED; exit 5; }}\n\
                 echo 'SHORTENED {i} PRESENT'\n\
                 else echo 'SHORTENED {i} ABSENT'; fi\n",
                kept = kept[i],
            ));
        }
        script.push_str(&format!(
            "if [ -f {conf} ]; then echo 'READY 1'; else echo 'READY 0'; fi\n",
            conf = super::shell_quote(self.conf_path),
        ));

        let out = self.forward(txn, &script).await?;
        let verdict = verdict(&out.stdout);
        if let Some(current) = conflict_in(verdict) {
            return Err(LimitError::Conflict {
                base: txn.base,
                current,
            });
        }
        let Some(had) = verdict.strip_prefix("READY ") else {
            return Err(LimitError::WriteFailed(unexpected("preparing", &out)));
        };
        let mut present = vec![false; txn.changes.len()];
        let mut made = vec![false; txn.dirs.len()];
        for line in out.stdout.lines() {
            let mut parts = line.split_whitespace();
            let (slots, yes) = match parts.next() {
                Some("SHORTENED") => (&mut present, "PRESENT"),
                Some("DIR") => (&mut made, "MADE"),
                _ => continue,
            };
            let index = parts.next().and_then(|i| i.parse::<usize>().ok());
            if let Some(slot) = index.and_then(|i| slots.get_mut(i)) {
                *slot = parts.next() == Some(yes);
            }
        }
        Ok(Prepared {
            had_conf: had.trim() == "1",
            present,
            made,
        })
    }

    /// Step 2: the new files, beside where they will go.
    ///
    /// Over SFTP, not as a marked step (T628): the SFTP server is not a process of this
    /// change and the holder cannot wait for it — but every name written here is unique to
    /// this change (`Txn::staged`), so a write still landing after the lock has passed on
    /// can only leave a file of ours lying about, never touch another change's.
    async fn stage(&self, txn: &Txn<'_>, rules: &str) -> Result<(), LimitError> {
        self.write_file(&self.staged_conf(txn), rules).await?;
        for change in txn.changes {
            self.write_file(&txn.staged(&change.path), &change.text)
                .await?;
        }
        Ok(())
    }

    fn staged_conf(&self, txn: &Txn<'_>) -> String {
        format!("{}.{}.tmp", self.conf_path, txn.id)
    }

    /// Step 3: keep the rules in force, then put the shortened descriptions and the new
    /// rules in place. Every copy and move is checked; the rules go in last.
    async fn swap(&self, txn: &Txn<'_>) -> Result<(), LimitError> {
        let conf = super::shell_quote(self.conf_path);
        let mut script = self.script_head(txn);
        // Never replace without a copy of what is replaced: a failed copy stops everything
        // before anything is moved.
        script.push_str(&format!(
            "if [ -f {conf} ]; then cp -p {conf} {previous} || {{ echo FAIL_KEEP_RULES; exit 5; }}; fi\n",
            previous = super::shell_quote(&self.previous_path()),
        ));
        for change in txn.changes {
            let path = super::shell_quote(&change.path);
            let staged = super::shell_quote(&txn.staged(&change.path));
            script.push_str(&format!(
                "chown {owner} {staged} 2>/dev/null || true\n\
                 chmod 644 {staged} || {{ echo FAIL_PUT_SHORTENED; exit 6; }}\n\
                 mv -f {staged} {path} || {{ echo FAIL_PUT_SHORTENED; exit 6; }}\n",
                owner = super::shell_quote(self.owner),
            ));
        }
        script.push_str(&format!(
            "mv -f {staged} {conf} || {{ echo FAIL_PUT_RULES; exit 7; }}\n\
             echo SWAPPED\n",
            staged = super::shell_quote(&self.staged_conf(txn)),
        ));

        let out = self.forward(txn, &script).await?;
        let verdict = verdict(&out.stdout);
        if verdict == "SWAPPED" && out.ok() {
            return Ok(());
        }
        if let Some(current) = conflict_in(verdict) {
            return Err(LimitError::Conflict {
                base: txn.base,
                current,
            });
        }
        Err(LimitError::WriteFailed(unexpected(
            "putting in place",
            &out,
        )))
    }

    /// After the rules went in and a check failed: put everything back, and make sure the
    /// serving answers again.
    async fn roll_back(&self, txn: &Txn<'_>, prepared: &Prepared) -> Result<(), LimitError> {
        self.put_back(txn, prepared)
            .await
            .map_err(LimitError::RollbackFailed)?;
        if !self.serving_answers().await {
            return Err(LimitError::RollbackFailed(String::from(
                "the previous configuration went back and the serving still does not answer",
            )));
        }
        Ok(())
    }

    /// Put back everything this change touched, whatever step it got to.
    ///
    /// **Never over somebody else's change.** The lock already means nobody else could have
    /// written in between; as defence in depth the rules are put back only if the file in
    /// force still carries the generation this change wrote — and left alone (with an
    /// error) if it carries neither that nor the one this change started from.
    ///
    /// **The generation only ever grows.** The rules come back with their old content but
    /// a **new** number (the one after this change's own), not their old one. A reader
    /// that saw this change's rules — `Serving::limits` reads without the lock — holds
    /// the number this change wrote; if the rollback handed back the number from before, a
    /// third change could write that same number again, and the reader's compare-and-swap
    /// against it would pass over the third change and bring back the rolled-back rule.
    ///
    /// The shortened descriptions come back as they were, or go if they were not there —
    /// and so do the directories this change made for them, so that everything under
    /// `_slow/` is as it was (T602). Nothing was removed before this point (removing is
    /// `sweep`, after success), so there is nothing removed to bring back. Staged files go
    /// in every case.
    async fn undo(&self, txn: &Txn<'_>, prepared: &Prepared) -> Result<(), String> {
        let conf = super::shell_quote(self.conf_path);
        let restored = super::shell_quote(&format!("{}.{}.undo.tmp", self.conf_path, txn.id));
        let next = txn.new_generation() + 1;
        let mut staged = vec![super::shell_quote(&self.staged_conf(txn))];
        for change in txn.changes {
            staged.push(super::shell_quote(&txn.staged(&change.path)));
        }
        // This change's own staged files go first and whatever else happens: their names are
        // unique to it, so removing them is safe even with the lock gone.
        let mut script = format!("rm -f {}\n", staged.join(" "));
        // Under the holder's mark rather than the door's (T628): a change whose client was
        // not heard from for a while may still put back what it did for as long as the
        // holder has not let the lock go — and it will not before this step has ended.
        script.push_str(&format!("{}\n", guard_held(txn)));
        script.push_str(&format!(
            "cur=$(grep -m1 '^# vrcast-generation ' {conf} 2>/dev/null | awk '{{print $3}}')\n\
             cur=${{cur:-0}}\n\
             swapped=0\n\
             if [ \"$cur\" = {new} ]; then\n",
            new = txn.new_generation(),
        ));
        if prepared.had_conf {
            script.push_str(&format!(
                "awk -v g={next} 'BEGIN{{d=0}} !d && /^# vrcast-generation /{{print \"# vrcast-generation \" g; d=1; next}} {{print}} END{{if(!d) print \"# vrcast-generation \" g}}' {previous} > {restored} \
                 || {{ rm -f {restored}; echo FAIL_RESTORE_RULES; exit 8; }}\n",
                previous = super::shell_quote(&self.previous_path()),
            ));
        } else {
            // There were no rules at all. An empty file would read as generation zero —
            // exactly the going-backwards this function must not do.
            script.push_str(&format!(
                "printf '# vrcast-generation %s\\n' {next} > {restored} \
                 || {{ rm -f {restored}; echo FAIL_RESTORE_RULES; exit 8; }}\n"
            ));
        }
        script.push_str(&format!(
            "mv -f {restored} {conf} || {{ rm -f {restored}; echo FAIL_RESTORE_RULES; exit 8; }}\n\
             swapped=1\n\
             elif [ \"$cur\" != {base} ]; then echo \"NOT_OURS $cur\"; exit 3; fi\n\
             fail=0\n",
            base = txn.base,
        ));
        let mut script_tail = String::new();
        for (i, change) in txn.changes.iter().enumerate() {
            let path = super::shell_quote(&change.path);
            let kept = super::shell_quote(&txn.kept(&change.path));
            if prepared.present.get(i).copied().unwrap_or(false) {
                script_tail.push_str(&format!(
                    "if [ -e {kept} ]; then mv -f {kept} {path} || fail=1; else fail=1; fi\n"
                ));
            } else {
                script_tail.push_str(&format!("rm -f {path} || fail=1\n"));
            }
        }
        // Then the directories this change made, deepest first. Only empty ones: one that
        // is not empty holds something that is not this change's, and is not ours to take.
        for (i, dir) in txn.dirs.iter().enumerate().rev() {
            if prepared.made.get(i).copied().unwrap_or(false) {
                script_tail.push_str(&format!(
                    "rmdir {} 2>/dev/null || true\n",
                    super::shell_quote(dir)
                ));
            }
        }
        script.push_str(&script_tail);
        script.push_str(&format!(
            "[ $fail = 0 ] || {{ echo FAIL_RESTORE_SHORTENED; exit 8; }}\n\
             if [ $swapped = 1 ]; then\n\
             caddy reload --config {main} --adapter caddyfile 2>&1 || {{ echo FAIL_RELOAD; exit 8; }}\n\
             fi\n\
             echo UNDONE\n",
            main = super::shell_quote(self.main_conf),
        ));

        let out = self
            .back(txn, &script)
            .await
            .map_err(|e| format!("putting back could not be run: {e}"))?;
        if verdict(&out.stdout) == "UNDONE" && out.ok() {
            return Ok(());
        }
        Err(unexpected("putting back", &out))
    }

    /// After success: the copies kept aside are no longer needed. `.previous` stays — it is
    /// the one copy a person can go back to by hand.
    async fn finish(&self, txn: &Txn<'_>) {
        self.discard_kept(txn).await;
    }

    /// Step 7 (T602): remove what the rules now in force no longer reach — only after they
    /// have been loaded and the serving has answered, and still under the lock.
    ///
    /// Named files are removed one by one and directories only when empty (`rmdir`): no
    /// recursive removal of a path worked out from a rule. `_slow/` itself is never among
    /// them. A failure here is logged and nothing more: the new rules are in force and
    /// checked, and a file nothing reaches does no harm (the next change removes it).
    async fn sweep(&self, txn: &Txn<'_>, plan: &slow_master::SlowPlan) {
        if plan.remove_files.is_empty() && plan.remove_dirs_if_empty.is_empty() {
            return;
        }
        let mut script = self.script_head_without_check(txn);
        script.push_str("fail=0\n");
        for file in &plan.remove_files {
            script.push_str(&format!("rm -f {} || fail=1\n", super::shell_quote(file)));
        }
        for dir in &plan.remove_dirs_if_empty {
            let dir = super::shell_quote(dir);
            script.push_str(&format!(
                "if [ -d {dir} ] && [ -z \"$(ls -A {dir} 2>/dev/null)\" ]; then rmdir {dir} || fail=1; fi\n"
            ));
        }
        script.push_str("[ $fail = 0 ] && echo SWEPT || echo SWEEP_INCOMPLETE\n");
        match self.forward(txn, &script).await {
            Ok(out) if verdict(&out.stdout) == "SWEPT" => {}
            outcome => tracing::warn!(
                ?outcome,
                "shortened descriptions no rule reaches any more were not all removed"
            ),
        }
    }

    /// Remove this change's own copies of the shortened descriptions. Their names are
    /// unique to this change, so nobody else's file can be among them.
    async fn discard_kept(&self, txn: &Txn<'_>) {
        let kept: Vec<String> = txn
            .changes
            .iter()
            .map(|c| super::shell_quote(&txn.kept(&c.path)))
            .collect();
        self.remove_own(txn, kept).await;
    }

    /// Remove, deepest first, the empty directories this change made (T602) — for when it
    /// stops without `undo`. A putting back like `undo`, and under the same mark (T628): a
    /// directory is shared by every change, and once the lock has passed on it may be the
    /// next change's.
    async fn unmake_dirs(&self, txn: &Txn<'_>, prepared: &Prepared) {
        let dirs: Vec<String> = txn
            .dirs
            .iter()
            .zip(&prepared.made)
            .rev()
            .filter(|(_, made)| **made)
            .map(|(dir, _)| format!("rmdir {} 2>/dev/null", super::shell_quote(dir)))
            .collect();
        if dirs.is_empty() {
            return;
        }
        // A putting back like `undo`, and behind the same barrier (T635).
        if let Err(e) = self.settle(txn).await {
            tracing::warn!(error = %e, "the directories this change made were left in place");
            return;
        }
        let script = format!("{}\n{}; true", guard_held(txn), dirs.join("; "));
        let _ = self.back(txn, &script).await;
    }

    /// Remove this change's own staged files, by the same reasoning.
    async fn discard_staged(&self, txn: &Txn<'_>) {
        let mut staged = vec![super::shell_quote(&self.staged_conf(txn))];
        for change in txn.changes {
            staged.push(super::shell_quote(&txn.staged(&change.path)));
        }
        self.remove_own(txn, staged).await;
    }

    /// Remove files whose names are this change's alone — with no check of the lock, since
    /// nobody else's file can be among them, but marked like every step (T628).
    async fn remove_own(&self, txn: &Txn<'_>, quoted: Vec<String>) {
        if quoted.is_empty() {
            return;
        }
        let outcome = self
            .forward(txn, &format!("rm -f {}", quoted.join(" ")))
            .await;
        if !matches!(&outcome, Ok(out) if out.ok()) {
            tracing::warn!(?outcome, "files of a change of the rules were not removed");
        }
    }

    /// The start of every step that writes: stop unless the lock is still ours, then check
    /// the generation.
    fn script_head(&self, txn: &Txn<'_>) -> String {
        let mut script = self.script_head_without_check(txn);
        script.push_str(&format!(
            "cur=$(grep -m1 '^# vrcast-generation ' {conf} 2>/dev/null | awk '{{print $3}}')\n\
             cur=${{cur:-0}}\n\
             if [ \"$cur\" != {base} ]; then echo \"CONFLICT $cur\"; exit 3; fi\n",
            conf = super::shell_quote(self.conf_path),
            base = txn.base,
        ));
        script
    }

    fn script_head_without_check(&self, txn: &Txn<'_>) -> String {
        format!("{}\n", guard(txn))
    }

    /// Ask the web server to check the configuration, then to take it.
    ///
    /// Both marked as steps of the change (T628). The reload also checks the door first: it
    /// makes the web server take what this change wrote, and a change nobody hears from
    /// must not do that — it fails (`LOST_LOCK`) and the change is put back instead.
    async fn check_and_reload(&self, txn: &Txn<'_>) -> Result<(), LimitError> {
        let validate = self
            .forward(
                txn,
                &format!(
                    "caddy validate --config {} --adapter caddyfile 2>&1",
                    super::shell_quote(self.main_conf)
                ),
            )
            .await?;
        if !validate.ok() {
            return Err(LimitError::ValidateFailed(last_words(&validate.stdout)));
        }

        let reload = self
            .forward(
                txn,
                &format!(
                    "{}\ncaddy reload --config {} --adapter caddyfile 2>&1",
                    guard(txn),
                    super::shell_quote(self.main_conf)
                ),
            )
            .await?;
        if !reload.ok() {
            return Err(LimitError::ReloadFailed(last_words(&reload.stdout)));
        }
        Ok(())
    }

    /// Ask the serving for something a viewer would ask for.
    ///
    /// From here, over the address a viewer uses — not from the server with a local
    /// request. A file can be on disk and readable and still not be served.
    async fn serving_answers(&self) -> bool {
        let Ok(client) = reqwest::Client::builder().timeout(ANSWER_TIMEOUT).build() else {
            return false;
        };
        match client.get(self.check_url).send().await {
            Ok(answer) => answer.status().is_success(),
            Err(_) => false,
        }
    }

    async fn write_file(&self, path: &str, body: &str) -> Result<(), LimitError> {
        let sftp = self.conn.sftp().await?;
        let written = async {
            let mut file = sftp.create(path.to_owned()).await?;
            file.write_all(body.as_bytes()).await?;
            file.flush().await?;
            file.shutdown().await?;
            Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
        }
        .await;
        written
            .map_err(|e| LimitError::Ssh(SshError::sftp(crate::store::redact::safe_display(&*e))))
    }

    /// Run one step of change `txn` on the server, carrying `mark` (`Txn::forward` or
    /// `Txn::back`) from its first instruction (`write_step`), so the lock's holder waits
    /// for it before it lets the lock go (T628).
    ///
    /// Every command a change runs under the lock after step 0 goes through here. What only
    /// reads — the rules and the quality sets read in step 0 — need not: nothing it does
    /// outlives it, and nothing it does can be mixed into another change.
    async fn step(&self, mark: &str, script: &str) -> Result<crate::ssh::CommandOutput, SshError> {
        self.conn
            .exec_with_timeout(&write_step(mark, script), step_ceiling())
            .await
    }

    /// A step going forward (`<id>.f`). One whose end was not heard (`unheard`) is noted on
    /// the change, so nothing is put back before the barrier has seen it end (T635).
    async fn forward(
        &self,
        txn: &Txn<'_>,
        script: &str,
    ) -> Result<crate::ssh::CommandOutput, SshError> {
        let outcome = self.step(&txn.forward(), script).await;
        if unheard(&outcome) {
            txn.unheard.store(true, std::sync::atomic::Ordering::SeqCst);
            tracing::warn!(
                ?outcome,
                "the end of a step of a change of the rules was not heard; it may still be \
                 running, and nothing is put back before it is seen to end"
            );
        }
        outcome
    }

    /// A step putting back (`<id>.u`).
    async fn back(
        &self,
        txn: &Txn<'_>,
        script: &str,
    ) -> Result<crate::ssh::CommandOutput, SshError> {
        self.step(&txn.back(), script).await
    }

    /// Put back everything this change touched — but only once no step of it going forward
    /// can still be running on the server (T635): `settle`, then `undo`.
    ///
    /// Before T635 the undo went in at once: a `caddy reload` of this change whose answer
    /// was lost (or that was given up on at `EXEC_CEILING`) could still be running, and take
    /// effect after the undo had put the old files back and reloaded them — Caddy's memory
    /// and the files on disk no longer the same (QA-21 №2).
    async fn put_back(&self, txn: &Txn<'_>, prepared: &Prepared) -> Result<(), String> {
        settle_then(|| self.settle(txn), || self.undo(txn, prepared)).await
    }

    /// The barrier before putting back (T635): nothing to do if every step going forward
    /// was heard to end; otherwise wait for every live `<id>.f` — `BARRIER_GRACE` for it to
    /// end on its own, then TERM and KILL, `BARRIER_ROUNDS` times at most — and go on only
    /// if the stop confirms nothing is left.
    ///
    /// Not confirmed — a step still alive after every round, an answer that is no answer,
    /// or the barrier itself not run — is an error, and nothing is put back: the caller
    /// reports `RollbackFailed` (`LIMITS_ROLLBACK_FAILED`). The lock stays with the holder,
    /// which goes on sending TERM and KILL for as long as the step is alive (T634).
    ///
    /// `/proc` that cannot be read is the one answer that lets the undo go ahead unconfirmed,
    /// as it did before T635 — the same rule the holder follows (owner's decision,
    /// 2026-09-30): on such a server nothing can ever be confirmed, and refusing every undo
    /// there would leave every failed change's rules in force.
    async fn settle(&self, txn: &Txn<'_>) -> Result<(), String> {
        if !txn.unheard.load(std::sync::atomic::Ordering::SeqCst) {
            return Ok(());
        }
        // The waits of the stop itself (5 s + 5 s a round, and a second between rounds) and
        // a minute for the scans and the link, on top of the grace.
        let ceiling =
            BARRIER_GRACE + Duration::from_secs(11) * BARRIER_ROUNDS + Duration::from_secs(60);
        let out = self
            .conn
            .exec_with_timeout(
                &barrier_command(&txn.forward(), BARRIER_GRACE.as_secs()),
                ceiling,
            )
            .await
            .map_err(|e| {
                format!("whether a step of this change is still running could not be asked: {e}")
            })?;
        match read_settled(&out.stdout) {
            Settled::Confirmed => {
                tracing::info!(said = %out.stdout.trim(), "no step of the change is running any more; putting back");
                Ok(())
            }
            Settled::Unreadable => {
                tracing::warn!(
                    "whether a step of the change is still running cannot be seen on this server \
                     (/proc unreadable); putting back unconfirmed, as before T635"
                );
                Ok(())
            }
            Settled::NotConfirmed(said) => Err(format!(
                "a step of this change may still be running on the server: {said}"
            )),
        }
    }
}

/// The line every step going forward starts with: go on only while the lock's door is open
/// — the client is still heard from (T618, T628).
///
/// Checked by the mark in the door's environment, not by the PID alone: a door that closed
/// (its channel closed, or it heard nothing for `LOCK_SILENCE`) may have had its PID given
/// to another process since. Runs after the step's own mark is set (`write_step`), so a
/// step that passes it is one the holder will wait for.
fn guard(txn: &Txn<'_>) -> String {
    alive_with(txn.holder.door, TXN_ENV, &txn.id)
}

/// The line a step putting back starts with: go on only while the holder still has not
/// begun its last wait — the lock is still this change's, and will stay so until this step
/// has ended (T628).
///
/// A change whose client was silent for longer than `LOCK_SILENCE` and then came back finds
/// the door closed; if the holder is still waiting for a step of the change, the lock is
/// still its own and it may put back what it did. Once the holder has let go (or dropped
/// its mark just before), the undo is refused (`LOST_LOCK`) — never silently, and never
/// over a change that may already have taken the lock.
fn guard_held(txn: &Txn<'_>) -> String {
    alive_with(txn.holder.held, HELD_ENV, &txn.id)
}

fn alive_with(pid: u32, var: &str, id: &str) -> String {
    format!(
        "tr '\\0' '\\n' < /proc/{pid}/environ 2>/dev/null | grep -qx '{var}={id}' \
         || {{ echo LOST_LOCK; exit 9; }}"
    )
}

/// Read up to the first newline, or to the end if there is none.
async fn first_line<R: tokio::io::AsyncRead + Unpin>(stream: &mut R) -> std::io::Result<String> {
    let mut line = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        if stream.read(&mut byte).await? == 0 || byte[0] == b'\n' {
            break;
        }
        line.push(byte[0]);
    }
    Ok(String::from_utf8_lossy(&line).into_owned())
}

/// The last line a step printed — each ends by saying how it went.
fn verdict(stdout: &str) -> &str {
    stdout
        .lines()
        .rev()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("")
}

fn conflict_in(verdict: &str) -> Option<u64> {
    verdict
        .strip_prefix("CONFLICT ")
        .map(|rest| rest.trim().parse().unwrap_or(u64::MAX))
}

fn unexpected(step: &str, out: &crate::ssh::CommandOutput) -> String {
    format!(
        "{step}: exit {:?}, said {:?}{}",
        out.exit_code,
        last_words(&out.stdout),
        if out.stderr.trim().is_empty() {
            String::new()
        } else {
            format!(", complained {:?}", last_words(&out.stderr))
        }
    )
}

fn parent_of(path: &str) -> &str {
    match path.rsplit_once('/') {
        Some(("", _)) => "/",
        Some((parent, _)) => parent,
        None => ".",
    }
}

/// The directories the descriptions go into, each once and every parent before its child:
/// `_slow/<slug>`, then `_slow/<slug>/<cap>` (T602).
///
/// `_slow/` itself is not among them: it belongs to the serving, not to any one change,
/// and is never removed — not even by the undoing of the first change that happened to
/// make it (`mkdir -p` makes it along the way). An empty `_slow/` is what a fresh server
/// has anyway, and the library already knows it as a service entry.
fn dirs_for(video_dir: &str, changes: &[Description]) -> Vec<String> {
    let slow_root = format!(
        "{}/{}",
        video_dir.trim_end_matches('/'),
        slow_master::SLOW_DIR
    );
    let mut dirs: Vec<String> = Vec::new();
    for change in changes {
        let cap_dir = parent_of(&change.path);
        let slug_dir = parent_of(cap_dir);
        // Only what the plan made, which is always inside `_slow/`.
        if parent_of(slug_dir) != slow_root {
            continue;
        }
        for dir in [slug_dir, cap_dir] {
            if !dirs.iter().any(|d| d == dir) {
                dirs.push(dir.to_owned());
            }
        }
    }
    dirs
}

/// The last few lines of a complaint — the part that says what is wrong.
///
/// Caddy prints its startup chatter before the actual objection, and a person shown the
/// chatter learns nothing.
fn last_words(text: &str) -> String {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    lines
        .iter()
        .rev()
        .take(3)
        .rev()
        .copied()
        .collect::<Vec<_>>()
        .join("\n")
}
