//! T168, T169, T170 — watching the viewers: both sources, and what they add up to.
//!
//! Two standing channels are held for as long as the watching goes on — one following the
//! access log, one polling the connection table (R-02, R-04). They are the two places set
//! aside by T153, and they are given back when the watch is dropped.
//!
//! The rules live in `domain::viewers`; here is only the fetching and the timing.

use std::sync::{Arc, Mutex};

use time::{Duration as TimeDuration, OffsetDateTime};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::domain::access_log;
use crate::domain::connections;
use crate::domain::viewers::{Session, VariantFacts, VariantLookup, Viewer};
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

/// What a running watch hands back on every change.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ViewersUpdate {
    pub server_id: String,
    pub active: Vec<Viewer>,
    /// How many are watching each medium — for the card in the library (FR-056).
    pub per_media: std::collections::HashMap<String, usize>,
}

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

/// A running watch. Dropping it stops both sources and gives the two channels back.
pub struct Watch {
    cancel: CancellationToken,
    session: Arc<Mutex<Session>>,
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
        self.session
            .lock()
            .map(|s| s.active(now))
            .unwrap_or_default()
    }

    /// Who watched earlier in this session.
    pub fn history(&self) -> Vec<Viewer> {
        self.session
            .lock()
            .map(|s| s.history().to_vec())
            .unwrap_or_default()
    }

    pub fn set_threshold(&self, threshold: TimeDuration) {
        if let Ok(mut session) = self.session.lock() {
            session.set_threshold(threshold);
        }
    }
}

/// Start watching.
///
/// `updates` is where the list goes on every change. The stream is deliberate rather than
/// the interface asking again and again: polling from the interface is what SC-009 exists
/// to prevent.
pub async fn start(
    conn: Connection,
    server_id: String,
    context: Arc<dyn ViewerContext>,
    threshold: TimeDuration,
    updates: mpsc::Sender<ViewersUpdate>,
) -> crate::ssh::Result<Watch> {
    let cancel = CancellationToken::new();
    let session = Arc::new(Mutex::new(Session::new(threshold)));

    // The log first. If the following will not start there is no point polling: the poll
    // alone can say somebody is pulling but never what.
    let mut lines = conn.stream_lines(&follow_command(), cancel.clone()).await?;

    // And it is waited for, not assumed — see FOLLOWING. Until this line comes back the
    // watching has not begun, whatever the call has returned.
    match tokio::time::timeout(FOLLOWING_STARTS_WITHIN, lines.recv()).await {
        Ok(Some(line)) if line.trim() == FOLLOWING => {}
        Ok(Some(other)) => {
            cancel.cancel();
            return Err(crate::ssh::SshError::Exec(format!(
                "following the serving's log answered with something unexpected: {other}"
            )));
        }
        Ok(None) | Err(_) => {
            cancel.cancel();
            return Err(crate::ssh::SshError::Exec(format!(
                "following the serving's log ({ACCESS_LOG_PATH}) would not start"
            )));
        }
    }

    {
        let session = session.clone();
        let context = context.clone();
        let cancel = cancel.clone();
        tokio::spawn(async move {
            loop {
                let line = tokio::select! {
                    biased;
                    () = cancel.cancelled() => break,
                    line = lines.recv() => line,
                };
                let Some(line) = line else { break };

                match access_log::parse_line(&line) {
                    Ok(request) => {
                        if let Ok(mut session) = session.lock() {
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

    {
        let session = session.clone();
        let context = context.clone();
        let cancel = cancel.clone();
        let conn = conn.clone();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(POLL_EVERY);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tokio::select! {
                    biased;
                    () = cancel.cancelled() => break,
                    _ = ticker.tick() => {}
                }

                let output = match conn.exec(&connections::poll_command()).await {
                    Ok(output) if output.ok() => output.stdout,
                    Ok(output) => {
                        tracing::debug!(
                            stderr = %crate::store::redact::scrub_viewer_addresses(output.stderr.trim()),
                            "the connection table would not be read"
                        );
                        continue;
                    }
                    Err(e) => {
                        // A break in the connection is not a reason to stop watching: it
                        // comes back, and the watching must come back with it rather than
                        // needing to be switched on again by hand.
                        tracing::debug!(error = %e, "the connection table could not be asked for");
                        continue;
                    }
                };

                let Some(poll) = connections::parse_poll(&output) else {
                    tracing::debug!("the connection table came back without a readable time");
                    continue;
                };

                let update = {
                    let Ok(mut session) = session.lock() else {
                        break;
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
                    let mut per_media = std::collections::HashMap::new();
                    for viewer in &active {
                        if let Some(id) = &viewer.media_id {
                            *per_media.entry(id.clone()).or_insert(0) += 1;
                        }
                    }
                    ViewersUpdate {
                        server_id: server_id.clone(),
                        active,
                        per_media,
                    }
                };

                if updates.send(update).await.is_err() {
                    // Nobody is listening any more — the window was closed. Holding two
                    // channels to talk to nobody would be the very leak T153 counts.
                    break;
                }
            }
        });
    }

    Ok(Watch { cancel, session })
}
