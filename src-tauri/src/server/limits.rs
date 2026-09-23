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

/// The longest the server keeps the lock for a change that never gave it back (T603).
///
/// The lock lives as long as one process on the server does, and that process ends the
/// moment our channel to it closes — a client that crashed, or a connection that dropped,
/// frees the lock by itself. This ceiling is for the one case that does not: a connection
/// that is neither alive nor closed, which the server may not notice for hours (the test
/// server's sshd has no `ClientAliveInterval`, and TCP keepalive defaults to two hours).
/// Well above `LOCK_WAIT` and seven times the measured worst case, so a healthy change
/// never runs into it; if one somehow did, every later step of it would notice the lock was
/// gone (`guard`) and stop rather than go on without it.
const LOCK_LEASE: Duration = Duration::from_secs(300);

/// How long giving the lock back is waited on before being left to the channel's closing.
const RELEASE_WAIT: Duration = Duration::from_secs(15);

/// The mark the lock's holder process carries in its environment, so a step of the change
/// can tell "the lock is held by us" from "the lock is held by somebody" or "a process with
/// the PID we were told now belongs to somebody else".
const TXN_ENV: &str = "VRCAST_LIMITS_TXN";

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
    /// Unique to this change: names its staged files and marks its lock holder.
    id: String,
    /// The PID of the process holding the lock for this change.
    holder_pid: u32,
    base: u64,
    changes: &'a [Description],
    /// The directories the descriptions go into, each once, every parent before its
    /// children (`_slow/<slug>` before `_slow/<slug>/<cap>`). Those this change had to make
    /// are removed again if it is undone.
    dirs: &'a [String],
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
/// Held by a process on the server — `flock` running `cat` on our channel — rather than by
/// a lock file we create and remove: that process ends the moment the channel closes, so a
/// client that crashed or a connection that dropped never leaves the lock behind. Dropping
/// this without `TxnLock::release` closes the channel (russh's `ChannelStream` closes it
/// on drop), which frees the lock the same way.
struct TxnLock {
    stream: russh::ChannelStream<russh::client::Msg>,
    holder_pid: u32,
    _permit: tokio::sync::OwnedSemaphorePermit,
}

impl TxnLock {
    /// Give the lock back, and wait until the server really has.
    ///
    /// End-of-input ends `cat`, `cat` ends `flock`, and only then does the server close its
    /// side of the channel — so the end of the channel means the lock is free, not that it
    /// is about to be.
    async fn release(mut self) {
        let _ = self.stream.shutdown().await;
        let mut rest = Vec::new();
        if tokio::time::timeout(RELEASE_WAIT, self.stream.read_to_end(&mut rest))
            .await
            .is_err()
        {
            tracing::warn!(
                "the server did not confirm the limits lock was given back; closing the channel"
            );
        }
    }
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
            .locked(&id, lock.holder_pid, limits, needs_ladder, base_generation)
            .await;
        lock.release().await;
        outcome
    }

    /// Step 0 and everything after it, the lock already held.
    async fn locked(
        &self,
        id: &str,
        holder_pid: u32,
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
            holder_pid,
            base: base_generation,
            changes: &changes,
            dirs: &dirs,
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
    /// The holder is `flock` itself, with our mark in its environment, running `cat` on
    /// this channel: `cat` ends on end-of-input — ours, or the server's when the connection
    /// dies — and `flock` with it. `timeout` bounds the rest (see `LOCK_LEASE`). The
    /// holder names its own PID (`$PPID` of the shell `flock` runs is `flock`) so later
    /// steps can check it is still there and still ours.
    ///
    /// The channel takes an ordinary place, not a standing one: it lasts as long as one
    /// change, and the watching of viewers keeps both of its places (T153). A change uses
    /// at most two places at once — this one and the step running under it.
    async fn lock(&self, id: &str) -> Result<TxnLock, LimitError> {
        let permit = self.conn.acquire_channel().await?;
        let channel = self.conn.open_session().await?;
        let command = format!(
            "env {TXN_ENV}={id} flock -x -w {wait} -E 75 {lock} \
             sh -c 'echo \"LOCKED $PPID\"; exec timeout {lease} cat >/dev/null' \
             || echo \"NOT_LOCKED $?\"",
            wait = LOCK_WAIT.as_secs(),
            lease = LOCK_LEASE.as_secs(),
            lock = super::shell_quote(&self.lock_path()),
        );
        channel
            .exec(true, command)
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
            // after that is given straight back (`cat` reads a closed input).
            Err(_) => return Err(LimitError::Busy),
        };

        if let Some(pid) = line.strip_prefix("LOCKED ") {
            if let Ok(holder_pid) = pid.trim().parse::<u32>() {
                return Ok(TxnLock {
                    stream,
                    holder_pid,
                    _permit: permit,
                });
            }
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
            return Err(match self.undo(txn, &prepared).await {
                Ok(()) => e,
                Err(undo) => LimitError::RollbackFailed(format!("{e}; then: {undo}")),
            });
        }

        if let Err(e) = self.check_and_reload().await {
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

        let out = self.conn.exec(&script).await?;
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

        let out = self.conn.exec(&script).await?;
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
        self.undo(txn, prepared)
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
        script.push_str(&self.script_head_without_check(txn));
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
            .conn
            .exec(&script)
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
        match self.conn.exec(&script).await {
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
        self.remove_own(kept).await;
    }

    /// Remove, deepest first, the empty directories this change made (T602) — for when it
    /// stops without `undo`.
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
        let _ = self.conn.exec(&format!("{}; true", dirs.join("; "))).await;
    }

    /// Remove this change's own staged files, by the same reasoning.
    async fn discard_staged(&self, txn: &Txn<'_>) {
        let mut staged = vec![super::shell_quote(&self.staged_conf(txn))];
        for change in txn.changes {
            staged.push(super::shell_quote(&txn.staged(&change.path)));
        }
        self.remove_own(staged).await;
    }

    async fn remove_own(&self, quoted: Vec<String>) {
        if quoted.is_empty() {
            return;
        }
        let outcome = self.conn.exec(&format!("rm -f {}", quoted.join(" "))).await;
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
    async fn check_and_reload(&self) -> Result<(), LimitError> {
        let validate = self
            .conn
            .exec(&format!(
                "caddy validate --config {} --adapter caddyfile 2>&1",
                super::shell_quote(self.main_conf)
            ))
            .await?;
        if !validate.ok() {
            return Err(LimitError::ValidateFailed(last_words(&validate.stdout)));
        }

        let reload = self
            .conn
            .exec(&format!(
                "caddy reload --config {} --adapter caddyfile 2>&1",
                super::shell_quote(self.main_conf)
            ))
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
}

/// The line every writing step starts with: go on only while our lock holder is alive.
///
/// Checked by the mark in the holder's environment, not by the PID alone: a holder that
/// ended (the lease ran out) may have had its PID given to another process since.
fn guard(txn: &Txn<'_>) -> String {
    format!(
        "tr '\\0' '\\n' < /proc/{pid}/environ 2>/dev/null | grep -qx '{TXN_ENV}={id}' \
         || {{ echo LOST_LOCK; exit 9; }}",
        pid = txn.holder_pid,
        id = txn.id,
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
