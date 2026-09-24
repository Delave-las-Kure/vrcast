//! T267 — the mechanism a deployment is made of.
//!
//! Every step is a pair: **check**, which says whether the thing is already so, and
//! **apply**, which makes it so. The check looks at the server rather than at a record of
//! what we did, and that one rule is where safety on a repeat comes from (FR-124, R-12) —
//! not from care taken inside each step.
//!
//! It buys three things at once:
//!
//! - a plan that can be shown before anything changes, because running every check and
//!   applying nothing is a plan (FR-122);
//! - a repeat after a failure that skips what is done (FR-124, SC-015);
//! - a way to ask whether an already deployed server still matches the reference — the same
//!   checks, again with nothing applied.
//!
//! **A step is done when its check says so, not when its apply returned.** The engine
//! re-runs the check after applying and calls the step failed if it still says no. That is
//! not belt and braces: on the live server the hardening step was written, ran without
//! complaint, and for six months `sshd -T` said password logins were on, while twenty-two
//! thousand attempts a day went at it. An apply that returns quietly proves nothing.

pub mod configs;
pub mod dns_check;
pub mod fail2ban;
pub mod firewall;
pub mod ipv6;
pub mod machine;
pub mod packages;
pub mod references;
pub mod services;
pub mod ssh_hardening;
pub mod ssh_key;
pub mod state_file;
pub mod swap;
pub mod tuning;
pub mod updates;
pub mod user_dirs;
pub mod verify;

use futures::future::BoxFuture;

pub use machine::Machine;

use crate::domain::deploy_steps::{self, Change, Checked, PlannedStep, Status, StepId, ORDER};
use crate::domain::dns_verdict::{Ipv6Choice, ServerAddresses};
use crate::domain::wording::Detail;
use crate::ssh::{Connection, SshError};

/// What every step is given.
///
/// Everything a step needs to know about *this* server and *this* person's choices. Nothing
/// here is global: two servers are deployed with two contexts, and a step that reached for a
/// setting instead of its context would work until somebody had a second server.
pub struct Context<'a> {
    pub conn: &'a Connection,
    /// The domain the serving will answer on.
    pub domain: &'a str,
    /// Where the videos live. From the profile — a server set up by hand keeps them where
    /// its owner put them (FR-004).
    pub video_dir: &'a str,
    /// What the person chose about IPv6 (FR-135).
    pub ipv6: Ipv6Choice,
    /// Where this machine is, as far as we know.
    pub server: ServerAddresses,
    /// The public half of the key to put on the server, in `authorized_keys` form.
    pub public_key: String,
    /// What the machine is like — memory, disk, interface, whether it is a container.
    /// Asked once before anything runs, so a step's `changes` can be exact without
    /// being able to ask anything.
    pub machine: Machine,
    /// The two things that cannot be found out from inside the connection we already
    /// have. See [`Proofs`].
    pub proofs: Proofs<'a>,
    /// Was this server already deployed by us?
    ///
    /// It changes one thing and it matters: on a bare machine a configuration file is
    /// simply written, and on a server already ours a file that differs from every
    /// version we ever wrote was **edited by somebody**, and must not be overwritten
    /// (the ownership rule of contracts/server-contract.md). A person who tuned their
    /// own web server and found the application had quietly undone it would be right
    /// to stop trusting it.
    pub already_ours: bool,
    /// This run's mark, and whether it has been asked to stop (T609). See [`RunMark`].
    pub run: RunMark,
}

/// The environment variable every remote command of one deployment or upgrade carries (T609).
///
/// Set on each command [`Context`] sends, and so inherited by everything the command starts —
/// `apt-get`, `dpkg`, the maintainer scripts, the lot. What is started through `systemctl`
/// lives under PID 1 and does not carry it, and the one process a step leaves behind on
/// purpose — the SSH undo timer — drops it explicitly (`env -u`), so a stop aimed at the mark
/// reaches exactly the run's own work and never its safety net.
pub const RUN_VAR: &str = "VRCAST_DEPLOY_RUN";

/// Every `apt-get` a step runs (T614): with retries on the downloads.
///
/// Five, because on 2026-09-23 archive.ubuntu.com answered 503 to part of nearly every batch
/// download while single requests went through — a deployment then failed at the first step
/// that installs anything, for a reason that a second try a moment later did not have.
pub const APT_GET: &str = "apt-get -o Acquire::Retries=5";

/// What every apt step runs before it installs anything (T609): finish whatever dpkg was in
/// the middle of when it was last interrupted.
///
/// A cancellation no longer interrupts dpkg — it waits for the command to end — but a broken
/// connection, a closed application or a machine restarted under a deployment still can, and
/// dpkg then refuses every later install with "dpkg was interrupted, you must manually run
/// 'dpkg --configure -a'" until somebody does it by hand (measured, T609 phase A: 6 and 230
/// packages left half-installed by a kill in unpack and in configure). Constitution V asks
/// that a repeated deployment finish the job without anybody's hands on the server; these two
/// lines are that, measured at 0.5–7.5 s after a kill and a fraction of a second on a server
/// with nothing to mend.
///
/// On somebody else's half-installed package they do the same — finish its configuration —
/// which is what apt itself would insist on before installing anything anyway.
///
/// **Only when dpkg has something unfinished** — `dpkg --audit` says so, or
/// `/var/lib/dpkg/updates` holds records of an interrupted run, which is what apt itself
/// checks before refusing with "dpkg was interrupted"; neither needs the lock to ask — and
/// **retried while dpkg's lock is busy**: unlike `apt-get` in its own frontend, a bare
/// `dpkg` does not wait for the lock, and on a freshly bought server `unattended-upgrades`
/// is often mid-run during the first deployment. A repair that failed at once on the lock
/// would be a new way for a healthy server's deployment to fail. Any other failure is not
/// retried: it is the answer, and the step says it. Two minutes of tries at most, so the
/// whole step still fits in one command's `EXEC_CEILING`.
pub const APT_HEAL: &str = "\
if [ -n \"$(dpkg --audit 2>/dev/null)\" ] || [ -n \"$(ls -A /var/lib/dpkg/updates 2>/dev/null)\" ]; then
  healed=0
  for _ in $(seq 1 24); do
    if said=$(dpkg --configure -a 2>&1); then healed=1; break; fi
    case \"$said\" in *lock*) sleep 5 ;; *) break ;; esac
  done
  if [ \"$healed\" != 1 ]; then
    printf '%s\\n' \"$said\" >&2
    echo 'E: dpkg --configure -a would not finish what an earlier install left undone' >&2
    exit 1
  fi
fi
apt-get -o Acquire::Retries=5 -f install -y -qq";

/// How many of apt's last lines an apt step's failure carries.
const APT_LINES: usize = 6;

/// What a failed apt step says (T614): apt's own last lines, errors first.
///
/// Its standard error when it wrote any — that is where `E:` lines go — and its standard
/// output otherwise; the last few lines, because the cause is at the end and the lines above
/// it are progress. Public so it can be checked without a server.
pub fn apt_complaint(said: &crate::ssh::CommandOutput) -> String {
    let source = if said.stderr.trim().is_empty() {
        &said.stdout
    } else {
        &said.stderr
    };
    let lines: Vec<&str> = source
        .lines()
        .map(str::trim_end)
        .filter(|l| !l.trim().is_empty())
        .collect();
    let tail = lines[lines.len().saturating_sub(APT_LINES)..].join("\n");
    let code = match said.exit_code {
        Some(c) => format!("exit {c}"),
        None => String::from("no exit status, the channel broke"),
    };
    if tail.is_empty() {
        format!("apt failed ({code}) and said nothing")
    } else {
        format!("apt failed ({code}): {tail}")
    }
}

/// One run's mark on the server, and its state on this side (T609).
///
/// **A cancellation does not interrupt a command; it stops the next one from starting.**
/// A command already on its way — `dpkg` unpacking, say — is waited for to its natural end:
/// killed halfway it leaves the package database interrupted (measured, T609 phase A), and a
/// repeat then fails until somebody runs `dpkg --configure -a` by hand. So once
/// [`RunMark::ask_to_stop`] is called, every further `ran`/`asks`/`put_file` answers
/// [`DeployError::Cancelled`] without sending anything, and the one in flight is let finish.
#[derive(Debug)]
pub struct RunMark {
    mark: String,
    stopping: std::sync::atomic::AtomicBool,
    /// When the command now running was sent, if one is — cleared only once the server has
    /// said how it ended. A command whose answer was lost with its channel may still be
    /// running, and is given until [`crate::ssh::exec::EXEC_CEILING`] after this moment.
    in_flight: std::sync::Mutex<Option<std::time::Instant>>,
}

impl RunMark {
    /// A mark for one run: unique, so a stop aimed at it cannot reach anybody else's run.
    pub fn fresh() -> Self {
        Self {
            mark: uuid::Uuid::new_v4().simple().to_string(),
            stopping: std::sync::atomic::AtomicBool::new(false),
            in_flight: std::sync::Mutex::new(None),
        }
    }

    pub fn as_str(&self) -> &str {
        &self.mark
    }

    /// No further command of this run is to start.
    pub fn ask_to_stop(&self) {
        self.stopping
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    pub fn is_stopping(&self) -> bool {
        self.stopping.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Until when the command last sent may legitimately still be running on the server —
    /// `None` when the server has already said it ended.
    pub fn may_run_until(&self) -> Option<std::time::Instant> {
        self.in_flight
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .map(|sent| sent + crate::ssh::exec::EXEC_CEILING)
    }

    /// The command as it is sent: carrying the mark, in a session and group of its own.
    ///
    /// `setsid -w` rather than `setsid`: it waits for the command and hands back its exit
    /// status, so the step reads the answer exactly as before. The command itself is run by
    /// the login shell (`$SHELL`), as it was before T609, so no step's text changes meaning.
    pub fn wrap(&self, command: &str) -> String {
        format!(
            "{RUN_VAR}={} setsid -w \"${{SHELL:-/bin/sh}}\" -c {}",
            crate::server::shell_quote(&self.mark),
            crate::server::shell_quote(command)
        )
    }
}

/// What has to be established by **opening a new connection**, not by reading a file.
///
/// This is the whole lesson of the hardening step. On the live server it was written,
/// it ran without complaint, and for six months the effective configuration said
/// password logins were allowed — while twenty-two thousand attempts a day went at it.
/// A file with the right line in it proves that the line is in the file.
///
/// Neither can be asked over the connection we are already using: it is open, it will
/// go on working whatever we do to the settings, and that is exactly what makes it the
/// wrong witness.
#[derive(Clone, Copy)]
pub struct Proofs<'a> {
    /// Does logging in **with our key** work, on a connection of its own?
    pub key_works: &'a (dyn Fn() -> BoxFuture<'a, bool> + Sync),
    /// Is logging in **with a password** actually refused?
    pub password_refused: &'a (dyn Fn() -> BoxFuture<'a, bool> + Sync),
}

impl Context<'_> {
    /// Send one command of this run: marked, in a group of its own (T609).
    ///
    /// Refuses to send anything once the run has been asked to stop — **the only way a
    /// cancellation acts on the server is by not starting the next command.** The one already
    /// running is never interrupted from here; see [`RunMark`].
    ///
    /// The time it was sent is kept until the server has said how it ended (an exit status
    /// came back). A command whose channel broke may still be running, and the stop that
    /// follows waits for it rather than signalling it at once.
    pub async fn exec_marked(&self, command: &str) -> Result<crate::ssh::CommandOutput> {
        if self.run.is_stopping() {
            return Err(DeployError::Cancelled);
        }
        self.send(command).await
    }

    /// The sending itself, whether or not a stop was asked — for the second half of an
    /// operation that must not be left halved ([`Self::put_file`]).
    async fn send(&self, command: &str) -> Result<crate::ssh::CommandOutput> {
        *self.run.in_flight.lock().unwrap_or_else(|e| e.into_inner()) =
            Some(std::time::Instant::now());
        let said = self.conn.exec(&self.run.wrap(command)).await?;
        if said.exit_code.is_some() {
            *self.run.in_flight.lock().unwrap_or_else(|e| e.into_inner()) = None;
        }
        Ok(said)
    }

    /// Run something on the server and hand back what it said.
    ///
    /// **A failed command hands back its complaint as well.** The applies below all end
    /// in `echo done` under `set -e`, so a command that dies part-way prints nothing at
    /// all to standard output — and the step then failed with an empty message. An empty
    /// reason is worse than a wrong one: there is nothing to look up, nothing to search
    /// for, and the person is left with the name of a step. Found by trimming a check and
    /// watching it fail with nothing to say (2026-08-27).
    pub async fn ran(&self, command: &str) -> Result<String> {
        let said = self.exec_marked(command).await?;
        if said.ok() || said.stderr.trim().is_empty() {
            return Ok(said.stdout);
        }
        Ok(format!("{}{}", said.stdout, said.stderr))
    }

    /// Install with apt, and fail with **apt's own words** when it fails (T614).
    ///
    /// `script` is the step's apt work, run under `set -e` after [`APT_HEAL`]; it must use
    /// [`APT_GET`] for every `apt-get`. On a non-zero exit the step's error carries the last
    /// lines apt wrote — "dpkg was interrupted", "Failed to fetch … 503", "has no
    /// installation candidate" — and not whatever the step would have tripped over next: a
    /// fail2ban step once reported "the sshd jail is not guarding" about a package that had
    /// never been installed (T609 phase A).
    pub async fn apt(&self, id: StepId, script: &str) -> Result<()> {
        let said = self
            .exec_marked(&format!(
                "set -e\nexport DEBIAN_FRONTEND=noninteractive\n{APT_HEAL}\n{script}\necho done"
            ))
            .await?;
        if said.ok() && said.stdout.lines().any(|l| l.trim() == "done") {
            return Ok(());
        }
        Err(DeployError::Step {
            id,
            detail: apt_complaint(&said),
            advice: None,
        })
    }

    /// Ask the server a yes-or-no question.
    ///
    /// The command must print `yes` or `no` rather than lean on its exit code: a command
    /// that is not there at all also exits non-zero, and "the check said no" would then
    /// mean either "it is not so" or "I could not ask" — which are different answers and
    /// only one of them is an answer.
    pub async fn asks(&self, command: &str) -> Result<bool> {
        // Standard output ONLY, unlike `ran`. A yes-or-no question that also carried the
        // command's complaints would answer "no" whenever anything on the way wrote a warning
        // — which is how, for a few minutes, a perfectly configured fail2ban was reported as
        // not installed (2026-08-27). What helps a failure's message ruins an answer.
        Ok(self.exec_marked(command).await?.stdout.trim() == "yes")
    }

    /// The answer for a step that cannot be settled in this environment (T246).
    pub fn not_here(&self, what: &str) -> Checked {
        Checked::NotPossibleHere {
            detail: self.machine.container_detail(what),
        }
    }

    /// Is the file on the server already exactly this?
    ///
    /// Compared by digest rather than by "does it exist": a configuration file that
    /// exists is not the same as the configuration we mean to deploy, and reading
    /// existence as done is how a server ends up running an older reference for ever.
    /// It is also what makes a repeat cheap — an unchanged file is not rewritten, and
    /// so the service is not reloaded for nothing.
    pub async fn file_is(&self, path: &str, body: &str) -> Result<bool> {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(body.as_bytes());
        let ours = hex::encode(hasher.finalize());
        let theirs = self
            .ran(&format!(
                "sha256sum -- {} 2>/dev/null | cut -d' ' -f1",
                crate::server::shell_quote(path)
            ))
            .await?;
        Ok(theirs.trim() == ours)
    }

    /// Put a file on the server, whole or not at all.
    ///
    /// Written beside and moved into place. A configuration written straight into its
    /// final path is readable half-written by whatever reloads next, and on a web
    /// server's main configuration that is the serving down rather than a bad edit.
    ///
    /// **One operation as far as a stop is concerned** (T609): refused before it starts once
    /// the run is stopping, and — once started — carried through to the move (or the tidying
    /// of the half-written copy) even if a stop is asked meanwhile. A stop between the two
    /// halves would leave a `*.vrcast.tmp` beside the real file for nothing.
    pub async fn put_file(&self, path: &str, body: &str) -> Result<()> {
        use tokio::io::AsyncWriteExt;

        if self.run.is_stopping() {
            return Err(DeployError::Cancelled);
        }
        let temp = format!("{path}.vrcast.tmp");
        let sftp = self.conn.sftp().await?;
        // `create` and not `write`: the library's `write` opens without creating, and on
        // a path that does not exist yet gives "no such file" — the name promises one
        // thing and does another (caught on a live server on 2026-08-25).
        let written = async {
            let mut file = sftp.create(temp.clone()).await?;
            file.write_all(body.as_bytes()).await?;
            file.flush().await?;
            file.shutdown().await?;
            Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
        }
        .await;

        if let Err(e) = written {
            let _ = self
                .send(&format!("rm -f -- {}", crate::server::shell_quote(&temp)))
                .await;
            return Err(DeployError::Ssh(crate::ssh::SshError::sftp(
                crate::store::redact::safe_display(&e),
            )));
        }

        // Its answer is not read, as `ran`'s was not before T609: the step's check after the
        // apply says whether the file is right, and that is what decides the step.
        self.send(&format!(
            "mv -f -- {} {}",
            crate::server::shell_quote(&temp),
            crate::server::shell_quote(path)
        ))
        .await?;
        Ok(())
    }
}

/// Function pointers rather than a trait: the trait would have to be dyn-compatible to live
/// in a list, which means either an extra crate or boxing every future by hand. This is the
/// same thing with less ceremony, and it keeps a step's two halves visibly one pair.
///
///
/// Generic over what a step is handed. The real steps take a [`Context`] with a live
/// connection in it; the checks of the mechanism itself hand over a note-taking stand-in,
/// and so can ask what happens when a step fails, or when its apply returns without
/// having done anything — **without a server**. A mechanism that could only be checked
/// through a server would count as unchecked (constitution, limits on how work is done),
/// and this is the piece every one of the fifteen steps rests on.
pub struct Step<C> {
    pub id: StepId,
    /// What this step will change on the server, for the plan a person agrees to (FR-122).
    pub changes: fn(&C) -> Vec<Change>,
    /// Is it already so? Looks at the server, never at a record of what we did.
    pub check: for<'a> fn(&'a C) -> BoxFuture<'a, Result<Checked>>,
    /// Make it so.
    pub apply: for<'a> fn(&'a C) -> BoxFuture<'a, Result<()>>,
}

pub type Result<T> = std::result::Result<T, DeployError>;

/// What went wrong, and where.
#[derive(Debug)]
pub enum DeployError {
    /// A step failed. Names it, because "the deployment failed" and "the firewall step
    /// failed" are different things to act on (FR-123).
    ///
    /// `advice` carries what the person should go and do about it, as a code with values
    /// rather than a sentence — the wordings live in the interface's dictionaries. The
    /// domain check is the step that has something to say here: "create an A record for
    /// this name with this value" is actionable, and "the deployment failed" is not.
    Step {
        id: StepId,
        detail: String,
        advice: Option<Detail>,
    },
    /// A step's apply returned without complaint and its check still says the thing is not
    /// so. Kept apart from an ordinary failure on purpose: this is the shape of the mistake
    /// that hid on the live server for six months, and a report that reads like any other
    /// failure would let it hide again.
    NotTaken {
        id: StepId,
    },
    Ssh(SshError),
    Cancelled,
}

impl From<SshError> for DeployError {
    fn from(e: SshError) -> Self {
        Self::Ssh(e)
    }
}

impl std::fmt::Display for DeployError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Step { id, detail, .. } => write!(f, "step {id:?} failed: {detail}"),
            Self::NotTaken { id } => write!(
                f,
                "step {id:?} reported success and its check still says it was not applied"
            ),
            Self::Ssh(e) => write!(f, "{e}"),
            Self::Cancelled => f.write_str("cancelled"),
        }
    }
}

impl std::error::Error for DeployError {}

/// Run every check and change nothing (FR-122).
///
/// This is what a person is shown before they agree, and it is also how an already deployed
/// server is compared against the reference — the same checks, and the difference is only
/// what is done with the answers.
pub async fn plan<C>(ctx: &C, steps: &[Step<C>]) -> Result<Vec<PlannedStep>> {
    let mut found = Vec::new();
    for step in in_order(steps) {
        found.push((step.id, (step.check)(ctx).await?));
    }
    let ids: Vec<StepId> = in_order(steps).iter().map(|s| s.id).collect();
    Ok(deploy_steps::plan(&ids, &found, |id| {
        changes_of(steps, id, ctx)
    }))
}

/// Carry the deployment out, reporting each step as it goes (FR-123).
///
/// `watch` is called when a step's outcome is settled — before the next one starts, so a
/// screen showing progress is never a step behind.
pub async fn run<C>(
    ctx: &C,
    steps: &[Step<C>],
    cancelled: &(dyn Fn() -> bool + Sync),
    watch: &mut (dyn FnMut(&PlannedStep) + Send),
) -> Result<Vec<PlannedStep>> {
    let mut done: Vec<PlannedStep> = Vec::new();

    for step in in_order(steps) {
        if cancelled() {
            return Err(DeployError::Cancelled);
        }

        let found = (step.check)(ctx).await?;
        let status = match found {
            // Already so. **This is the whole of safety on a repeat**: a run after a failure
            // does not undo the half that succeeded, and nothing had to be remembered between
            // the two runs for that to hold.
            Checked::Applied => Status::Applied,
            Checked::NotNeeded => Status::Skipped {
                why: deploy_steps::SkipReason::NotNeeded,
            },
            Checked::NotPossibleHere { detail } => Status::Skipped {
                why: deploy_steps::SkipReason::NotPossibleHere { detail },
            },
            Checked::NotApplied => match (step.apply)(ctx).await {
                Ok(()) => {
                    // **Asked again, on purpose.** A step is done when the server says so,
                    // not when our own code returned. The one time this rule was missing, the
                    // hardening step ran without complaint and password logins stayed on for
                    // half a year.
                    match (step.check)(ctx).await? {
                        Checked::Applied => Status::Applied,
                        Checked::NotNeeded => Status::Skipped {
                            why: deploy_steps::SkipReason::NotNeeded,
                        },
                        Checked::NotPossibleHere { detail } => Status::Skipped {
                            why: deploy_steps::SkipReason::NotPossibleHere { detail },
                        },
                        Checked::NotApplied => {
                            let planned = settled(
                                steps,
                                step.id,
                                ctx,
                                &Status::Failed {
                                    detail: String::from("applied, and the check still says no"),
                                },
                            );
                            watch(&planned);
                            done.push(planned);
                            return Err(DeployError::NotTaken { id: step.id });
                        }
                    }
                }
                Err(DeployError::Cancelled) => {
                    // Asked to stop while this step was under way: its command in flight was
                    // let finish and the next one was not sent (T609). Not a failure of the
                    // step — nothing about the server said no — so it is not reported as one,
                    // and a repeat asks its check afresh.
                    return Err(DeployError::Cancelled);
                }
                Err(e) => {
                    let detail = e.to_string();
                    let planned = settled(
                        steps,
                        step.id,
                        ctx,
                        &Status::Failed {
                            detail: detail.clone(),
                        },
                    );
                    watch(&planned);
                    done.push(planned);
                    // A blocking step that failed ends the run. Going on would apply the rest
                    // to a server missing what they need, and every failure after it would say
                    // "missing" — a page of consequences with the cause five screens up.
                    if deploy_steps::stops_the_run(step.id) {
                        // The step's own error is handed back untouched rather than rebuilt
                        // from its text. The domain check puts what to do about it in there
                        // — which record to create, with what value — and a rebuilt error
                        // would arrive with that advice quietly missing.
                        return Err(e);
                    }
                    continue;
                }
            },
        };

        let planned = settled(steps, step.id, ctx, &status);
        watch(&planned);
        done.push(planned);
    }

    Ok(done)
}

/// The steps in the deployment's own order, whatever order they were handed in.
///
/// The order is not a preference in three places (R-12), and a caller building the list by
/// hand is exactly where it would be got wrong — so it is imposed here rather than trusted.
fn in_order<C>(steps: &[Step<C>]) -> Vec<&Step<C>> {
    ORDER
        .iter()
        .filter_map(|id| steps.iter().find(|s| s.id == *id))
        .collect()
}

fn changes_of<C>(steps: &[Step<C>], id: StepId, ctx: &C) -> Vec<Change> {
    steps
        .iter()
        .find(|s| s.id == id)
        .map(|s| (s.changes)(ctx))
        .unwrap_or_default()
}

fn settled<C>(steps: &[Step<C>], id: StepId, ctx: &C, status: &Status) -> PlannedStep {
    PlannedStep {
        id,
        changes: changes_of(steps, id, ctx)
            .iter()
            .map(deploy_steps::Change::detail)
            .collect(),
        blocking: deploy_steps::blocking(id),
        status: status.clone(),
    }
}

/// The whole deployment.
///
/// Listed in the order it runs in, though the engine imposes that anyway ([`in_order`]) — a
/// list assembled by hand is exactly where the order would be got wrong, and the three
/// mandatory pairs are each one that fails without failing.
pub fn all<'a>() -> Vec<Step<Context<'a>>> {
    vec![
        dns_check::step(),
        swap::step(),
        packages::step(),
        user_dirs::step(),
        configs::step(),
        services::step(),
        ssh_key::step(),
        ssh_hardening::step(),
        firewall::step(),
        ipv6::step(),
        fail2ban::step(),
        updates::step(),
        tuning::step(),
        verify::step(),
        state_file::step(),
    ]
}
