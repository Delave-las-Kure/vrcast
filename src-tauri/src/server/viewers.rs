//! T168, T169, T170 — watching the viewers: both sources, and what they add up to.
//!
//! Two standing channels are held for as long as the watching goes on — one following the
//! access log, one polling the connection table (R-02, R-04). They are the two places set
//! aside by T153, and they are given back when the watch is dropped.
//!
//! The rules live in `domain::viewers`; here is only the fetching and the timing.
//!
//! **T664 — and the getting back.** A connection that dies takes both sources with it.
//! ⚠ QA-24B-05: the reader of the log used to end with nothing to follow it, the poll went on
//! failing quietly through the same dead connection, and the screen showed the last list as
//! if it were now — for good. Now the watching notices (the log's reader ends, or a poll does
//! not answer within [`POLL_ANSWER_WITHIN`]), says so on the next update
//! ([`WatchState::Reconnecting`], with the last list and how old it is), and tries again with
//! a growing pause (`domain::viewers::backoff`): a new connection and both sources on it,
//! until it is back, it is stopped, or the failure is one trying again cannot cure.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use futures::future::BoxFuture;
use time::{Duration as TimeDuration, OffsetDateTime};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::domain::access_log;
use crate::domain::connections;
use crate::domain::viewers::{
    Session, VariantFacts, VariantLookup, Viewer, WatchState, WatchStatus, BACKOFF_CEILING,
};
use crate::ssh::Connection;

/// Where the serving writes what it served (`contracts/server-contract.md`).
pub const ACCESS_LOG_PATH: &str = "/var/log/caddy/access.log";

/// How often the connection table is asked for.
///
/// **Three seconds.** R-02 allows two to five; SC-005 requires a viewer to show up within
/// ten. The choice is bounded from both sides and is not a matter of taste: the poll holds
/// one of eight channels for the whole session (R-04), so going faster costs the rest of
/// the application, while going slower eats into the ten seconds — a new viewer waits for
/// the first poll, and then for a second one before any speed can be worked out at all.
/// Three leaves room under the limit and puts two polls inside the first ten seconds.
pub const POLL_EVERY: std::time::Duration = std::time::Duration::from_secs(3);

/// What the following says before it starts, so that it can be waited for.
///
/// Not decoration. Asking the server to run something comes back as soon as the request has
/// gone out, and the command itself starts a moment later. `tail -n 0` begins at the end of
/// the file, so everything served inside that moment is served as far as it is concerned —
/// missed, with nothing to show that anything was. On a busy machine the moment is long
/// enough to swallow a viewer's first request, which for a directly served film is the only
/// one there will be for the whole showing: they would appear in the list watching an
/// unknown something and stay that way. Caught on 2026-08-26, when the check for exactly
/// that case failed every time and looked like a fault in the parsing.
pub const FOLLOWING: &str = "vrcast-following-now";

/// The command that follows the log.
///
/// `-F` rather than `-f`: it follows the **name**, so when the serving rotates the file the
/// following moves to the new one. With `-f` it would go on holding a file nobody writes to
/// any more, and the list of viewers would quietly stop changing — quietly being the whole
/// problem, since there is no error to notice.
///
/// `-n 0` means start from the end. What was served before the watching began has been
/// served; showing it as current viewing would fill the list with people who left hours
/// ago.
///
/// `exec` so that no shell is left waiting behind the following: one process on the server
/// instead of two, and the one that gets signalled is the one doing the work.
pub fn follow_command() -> String {
    format!(
        "echo {FOLLOWING}; exec tail -n 0 -F {} 2>/dev/null",
        super::shell_quote(ACCESS_LOG_PATH)
    )
}

/// How long to wait for the following to say it has started.
///
/// Generous: the server may be busy, and giving up early would mean watching nobody at all.
const FOLLOWING_STARTS_WITHIN: std::time::Duration = std::time::Duration::from_secs(20);

/// How long a poll of the connection table may take before the connection counts as lost.
///
/// ⚠ **Not left to the SSH keepalive.** A cut network does not close a connection: it goes
/// silent, and russh's keepalive notices only after 90–120 seconds (measured for T560). For
/// all that time the watching would go on "watching" — nothing comes, nothing fails. `ss` on
/// a live server answers in milliseconds; fifteen seconds is generous for a loaded one and
/// still inside the thirty-second activity threshold.
pub const POLL_ANSWER_WITHIN: std::time::Duration = std::time::Duration::from_secs(15);

/// How many polls in a row may fail on a connection that still looks alive before it is
/// counted as lost anyway. One is a hiccup; three in a row (nine seconds) is not.
const POLL_FAILURES_TOLERATED: u32 = 3;

/// How long one try at getting the watching back may take — a new connection, a login, and
/// the following of the log starting on it. A connect into a silent network otherwise waits
/// for the operating system's own timeout.
pub const RECONNECT_WITHIN: std::time::Duration = std::time::Duration::from_secs(45);

/// What a running watch hands back on every change.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ViewersUpdate {
    pub server_id: String,
    pub active: Vec<Viewer>,
    /// How many are watching each medium — for the card in the library (FR-056).
    pub per_media: HashMap<String, usize>,
    /// Whether this list is current (T664). While `reconnecting` it is the last list there
    /// was, and `as_of` says how old it is.
    #[serde(default = "watching")]
    pub watch: WatchState,
    /// When the list was last current, by this machine's clock. `None` before the first.
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub as_of: Option<OffsetDateTime>,
    /// How many tries at getting the watching back have been made. Zero while watching.
    #[serde(default)]
    pub attempt: u32,
}

fn watching() -> WatchState {
    WatchState::Watching
}

/// Why a try at getting the connection back did not work.
#[derive(Debug, Clone)]
pub enum Retry {
    /// Worth trying again: the server did not answer, the network is down.
    Transient(String),
    /// Trying again cannot help: the key changed, the login is refused, the profile is gone.
    /// The watching stops and says so.
    Permanent(String),
}

/// Where a new connection comes from when the old one is lost.
///
/// Given from outside for the same reason the context is: this layer does not read
/// profiles or secrets.
pub type Reconnect =
    Arc<dyn Fn() -> BoxFuture<'static, Result<Connection, Retry>> + Send + Sync + 'static>;

/// Where the facts about a variant come from, and where an address is placed.
///
/// Given from outside so that this layer neither reads the library nor opens the table of
/// places: what it does is fetch and time.
pub trait ViewerContext: Send + Sync + 'static {
    fn facts(&self, asked: &access_log::Asked) -> VariantFacts;
    fn place(&self, ip: &str) -> crate::domain::geo::Place;
}

/// Lets a context stand where the rules expect somewhere to ask.
///
/// A blanket implementation would have been shorter and does not work: the rules already
/// accept a plain closure — which is what makes them checkable without any of this — and
/// the compiler cannot know that no context will ever also be a closure.
struct AsLookup<'a>(&'a dyn ViewerContext);

impl VariantLookup for AsLookup<'_> {
    fn facts(&self, asked: &access_log::Asked) -> VariantFacts {
        self.0.facts(asked)
    }
}

/// What the watching shares between its own task and whoever holds the [`Watch`].
struct Shared {
    server_id: String,
    session: Mutex<Session>,
    status: Mutex<WatchStatus>,
    /// The last list there was — shown, marked, while the watching is being got back.
    last: Mutex<(Vec<Viewer>, HashMap<String, usize>)>,
}

impl Shared {
    fn status(&self) -> WatchStatus {
        self.status
            .lock()
            .map(|s| s.clone())
            .unwrap_or_else(|_| WatchStatus::started())
    }

    fn update(&self) -> ViewersUpdate {
        let (active, per_media) = self.last.lock().map(|l| l.clone()).unwrap_or_default();
        let status = self.status();
        ViewersUpdate {
            server_id: self.server_id.clone(),
            active,
            per_media,
            watch: status.state,
            as_of: status.last_snapshot_at,
            attempt: status.attempt,
        }
    }

    fn with_status<T>(&self, f: impl FnOnce(&mut WatchStatus) -> T) -> Option<T> {
        self.status.lock().ok().map(|mut s| f(&mut s))
    }
}

/// A running watch. Dropping it stops both sources and gives the two channels back.
pub struct Watch {
    cancel: CancellationToken,
    shared: Arc<Shared>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Watch {
    fn drop(&mut self) {
        // Not left to whoever holds it: stopping happens in several places — the server was
        // switched, the window was closed — and a stop by hand would be forgotten in one of
        // them. Two channels held for ever would then be found much later, as a third one
        // failing to open.
        self.cancel.cancel();
    }
}

impl Watch {
    /// Who is watching now, by the server's clock as last read.
    pub fn active(&self, now: OffsetDateTime) -> Vec<Viewer> {
        self.shared
            .session
            .lock()
            .map(|s| s.active(now))
            .unwrap_or_default()
    }

    /// Who watched earlier in this session.
    pub fn history(&self) -> Vec<Viewer> {
        self.shared
            .session
            .lock()
            .map(|s| s.history().to_vec())
            .unwrap_or_default()
    }

    pub fn set_threshold(&self, threshold: TimeDuration) {
        if let Ok(mut session) = self.shared.session.lock() {
            session.set_threshold(threshold);
        }
    }

    /// Where the watching stands.
    pub fn status(&self) -> WatchStatus {
        self.shared.status()
    }

    /// Whether this watch is still doing its job — watching, or getting the watching back.
    ///
    /// ⚠ **QA-24B-05.** A repeated `viewers_watch_start` used to answer "already watching"
    /// by the server's name alone, over a watch whose sources had long since died. What is
    /// asked now is whether its own task is still running and has not given up.
    pub fn is_alive(&self) -> bool {
        !self.task.is_finished()
            && !self.cancel.is_cancelled()
            && self.status().state != WatchState::Stopped
    }

    /// The list and the state as they stand, for a screen that has just opened — without it
    /// the screen would wait for the next poll (or, while reconnecting, for the next try)
    /// to learn anything at all.
    pub fn current(&self) -> ViewersUpdate {
        self.shared.update()
    }
}

/// Start following the log on `conn`, and wait until it has really started.
async fn follow(
    conn: &Connection,
    life: CancellationToken,
) -> crate::ssh::Result<mpsc::Receiver<String>> {
    let mut lines = conn.stream_lines(&follow_command(), life.clone()).await?;

    // And it is waited for, not assumed — see FOLLOWING. Until this line comes back the
    // watching has not begun, whatever the call has returned.
    match tokio::time::timeout(FOLLOWING_STARTS_WITHIN, lines.recv()).await {
        Ok(Some(line)) if line.trim() == FOLLOWING => Ok(lines),
        Ok(Some(other)) => {
            life.cancel();
            Err(crate::ssh::SshError::Exec(format!(
                "following the serving's log answered with something unexpected: {other}"
            )))
        }
        Ok(None) | Err(_) => {
            life.cancel();
            Err(crate::ssh::SshError::Exec(format!(
                "following the serving's log ({ACCESS_LOG_PATH}) would not start"
            )))
        }
    }
}

/// Start watching.
///
/// `updates` is where the list goes on every change. The stream is deliberate rather than
/// the interface asking again and again: polling from the interface is what SC-009 exists
/// to prevent.
///
/// Without a way to open a new connection, a lost source is got back on the same connection
/// while it is alive; once the connection itself is gone the watching stops and says so.
/// The application always gives one — see [`start_reconnecting`].
pub async fn start(
    conn: Connection,
    server_id: String,
    context: Arc<dyn ViewerContext>,
    threshold: TimeDuration,
    updates: mpsc::Sender<ViewersUpdate>,
) -> crate::ssh::Result<Watch> {
    start_reconnecting(conn, None, server_id, context, threshold, updates).await
}

/// Start watching, and get it back through `reconnect` whenever the connection is lost.
///
/// The first start is answered to whoever asked, as before: a server that will not let the
/// watching begin is an error on the screen, not a quiet retry. Only a watch that was
/// running is got back.
pub async fn start_reconnecting(
    conn: Connection,
    reconnect: Option<Reconnect>,
    server_id: String,
    context: Arc<dyn ViewerContext>,
    threshold: TimeDuration,
    updates: mpsc::Sender<ViewersUpdate>,
) -> crate::ssh::Result<Watch> {
    let cancel = CancellationToken::new();

    // The log first. If the following will not start there is no point polling: the poll
    // alone can say somebody is pulling but never what.
    let life = cancel.child_token();
    let lines = follow(&conn, life.clone()).await?;

    let shared = Arc::new(Shared {
        server_id,
        session: Mutex::new(Session::new(threshold)),
        status: Mutex::new(WatchStatus::started()),
        last: Mutex::new(Default::default()),
    });

    let task = tokio::spawn(supervise(
        Stretch { conn, lines, life },
        reconnect,
        context,
        shared.clone(),
        cancel.clone(),
        updates,
    ));

    Ok(Watch {
        cancel,
        shared,
        task,
    })
}

/// One connection and the following of the log on it: what one stretch of watching runs on.
struct Stretch {
    conn: Connection,
    lines: mpsc::Receiver<String>,
    /// Ends this stretch — both sources — without ending the watching.
    life: CancellationToken,
}

/// How one stretch of watching ended.
enum End {
    /// Stopped from outside, or nobody is listening any more.
    Over,
    /// The connection or a source was lost.
    Lost,
}

async fn supervise(
    mut stretch: Stretch,
    reconnect: Option<Reconnect>,
    context: Arc<dyn ViewerContext>,
    shared: Arc<Shared>,
    cancel: CancellationToken,
    updates: mpsc::Sender<ViewersUpdate>,
) {
    loop {
        let Stretch { conn, lines, life } = stretch;
        if let End::Over = live(&conn, lines, &life, &context, &shared, &cancel, &updates).await {
            life.cancel();
            return;
        }
        life.cancel();
        shared.with_status(WatchStatus::lost);
        tracing::info!(
            server = %shared.server_id,
            "the watching of viewers lost its connection; getting it back"
        );
        if updates.send(shared.update()).await.is_err() {
            return;
        }

        // Getting it back: a pause that grows, then a connection and both sources on it.
        stretch = loop {
            let wait = shared
                .with_status(WatchStatus::next_try)
                .unwrap_or(BACKOFF_CEILING);
            tokio::select! {
                biased;
                () = cancel.cancelled() => return,
                () = tokio::time::sleep(wait) => {}
            }

            let next_life = cancel.child_token();
            let attempt = tokio::time::timeout(
                RECONNECT_WITHIN,
                reopen(&conn, reconnect.as_ref(), next_life.clone()),
            );
            let outcome = tokio::select! {
                biased;
                () = cancel.cancelled() => { next_life.cancel(); return }
                outcome = attempt => outcome,
            };
            match outcome {
                Ok(Ok((new_conn, lines, fresh))) => {
                    if fresh {
                        // The old connection is let go politely, and not waited on: one
                        // into a silent network may take its time to say goodbye.
                        let old = conn.clone();
                        tokio::spawn(async move {
                            let _ = tokio::time::timeout(
                                std::time::Duration::from_secs(5),
                                old.close(),
                            )
                            .await;
                        });
                    }
                    tracing::info!(server = %shared.server_id, "the watching of viewers is back");
                    break Stretch {
                        conn: new_conn,
                        lines,
                        life: next_life,
                    };
                }
                Ok(Err(Retry::Permanent(why))) => {
                    next_life.cancel();
                    tracing::warn!(
                        server = %shared.server_id,
                        why = %crate::store::redact::redact(&why),
                        "the watching of viewers cannot be got back"
                    );
                    shared.with_status(|s| s.try_failed(true));
                    let _ = updates.send(shared.update()).await;
                    return;
                }
                Ok(Err(Retry::Transient(why))) => {
                    next_life.cancel();
                    tracing::debug!(
                        why = %crate::store::redact::redact(&why),
                        "the watching of viewers is not back yet"
                    );
                    shared.with_status(|s| s.try_failed(false));
                }
                Err(_) => {
                    next_life.cancel();
                    tracing::debug!("a try at getting the watching back took too long");
                    shared.with_status(|s| s.try_failed(false));
                }
            }
            // The count of tries has moved: the screen says how many.
            if updates.send(shared.update()).await.is_err() {
                return;
            }
        };
    }
}

/// A connection and the following of the log on it, for the next stretch of watching.
/// The flag says whether the connection is a new one (and the old one should be let go).
async fn reopen(
    old: &Connection,
    reconnect: Option<&Reconnect>,
    life: CancellationToken,
) -> Result<(Connection, mpsc::Receiver<String>, bool), Retry> {
    let (conn, fresh) = match reconnect {
        Some(reconnect) => (reconnect().await?, true),
        None if old.is_alive() => (old.clone(), false),
        None => {
            return Err(Retry::Permanent(String::from(
                "the connection is gone and there is no way to open another",
            )))
        }
    };
    match follow(&conn, life).await {
        Ok(lines) => Ok((conn, lines, fresh)),
        Err(e) => {
            if fresh {
                let _ = tokio::time::timeout(std::time::Duration::from_secs(5), conn.close()).await;
            }
            Err(Retry::Transient(e.to_string()))
        }
    }
}

/// One stretch of watching on one connection, until it is stopped or lost.
async fn live(
    conn: &Connection,
    mut lines: mpsc::Receiver<String>,
    life: &CancellationToken,
    context: &Arc<dyn ViewerContext>,
    shared: &Arc<Shared>,
    cancel: &CancellationToken,
    updates: &mpsc::Sender<ViewersUpdate>,
) -> End {
    {
        let shared = shared.clone();
        let context = context.clone();
        let life = life.clone();
        tokio::spawn(async move {
            loop {
                let line = tokio::select! {
                    biased;
                    () = life.cancelled() => return,
                    line = lines.recv() => line,
                };
                // The following ended by itself: the channel closed under it — the
                // connection went, or `tail` was killed. Either way the log is no longer
                // being read, and this whole stretch is over (T664). It used to end here
                // quietly, and the list went on as if the log were still being followed.
                let Some(line) = line else {
                    life.cancel();
                    return;
                };

                match access_log::parse_line(&line) {
                    Ok(request) => {
                        if let Ok(mut session) = shared.session.lock() {
                            session.note_request(&request, &AsLookup(context.as_ref()));
                        }
                    }
                    // A line caught mid-write, or one of the serving's own notes. Both are
                    // normal and neither is worth a word in the log — at several a second
                    // they would bury everything else.
                    Err(_) => continue,
                }
            }
        });
    }

    let ended = || {
        if cancel.is_cancelled() {
            End::Over
        } else {
            End::Lost
        }
    };

    let mut ticker = tokio::time::interval(POLL_EVERY);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut failures = 0u32;
    loop {
        tokio::select! {
            biased;
            () = life.cancelled() => return ended(),
            _ = ticker.tick() => {}
        }

        let command = connections::poll_command();
        let asked = tokio::time::timeout(POLL_ANSWER_WITHIN, conn.exec(&command));
        let answer = tokio::select! {
            biased;
            () = life.cancelled() => return ended(),
            answer = asked => answer,
        };

        let output = match answer {
            Ok(Ok(output)) if output.ok() => output.stdout,
            Ok(Ok(output)) => {
                // The command ran and said no: the connection itself is fine.
                tracing::debug!(
                    stderr = %crate::store::redact::scrub_viewer_addresses(output.stderr.trim()),
                    "the connection table would not be read"
                );
                continue;
            }
            Ok(Err(e)) => {
                // Once is a hiccup. A connection that is gone, or one that keeps failing,
                // is lost — and waiting on it for ever is exactly what QA-24B-05 found.
                tracing::debug!(error = %e, "the connection table could not be asked for");
                failures += 1;
                if !conn.is_alive() || failures >= POLL_FAILURES_TOLERATED {
                    return ended();
                }
                continue;
            }
            Err(_) => {
                tracing::debug!("the connection table did not answer in time");
                return ended();
            }
        };
        failures = 0;

        let Some(poll) = connections::parse_poll(&output) else {
            tracing::debug!("the connection table came back without a readable time");
            continue;
        };

        {
            let Ok(mut session) = shared.session.lock() else {
                return End::Over;
            };
            session.note_connections(&poll.rows, poll.at);

            // Where the new addresses are. Looked up here rather than on every
            // refresh: the answer does not change, and the table is large.
            for ip in session.without_place() {
                let place = context.place(&ip);
                if !place.is_empty() {
                    session.note_place(&ip, place.country, place.city, place.asn_org);
                }
            }

            session.retire_gone(poll.at);
            let active = session.active(poll.at);
            let mut per_media = HashMap::new();
            for viewer in &active {
                if let Some(id) = &viewer.media_id {
                    *per_media.entry(id.clone()).or_insert(0) += 1;
                }
            }
            if let Ok(mut last) = shared.last.lock() {
                *last = (active, per_media);
            }
        }
        shared.with_status(|s| s.snapshot(OffsetDateTime::now_utc()));

        if updates.send(shared.update()).await.is_err() {
            // Nobody is listening any more — the window was closed. Holding two
            // channels to talk to nobody would be the very leak T153 counts.
            return End::Over;
        }
    }
}
