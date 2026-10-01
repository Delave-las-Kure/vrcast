//! T171, T172 — the commands for watching viewers.
//!
//! The contract: `contracts/ipc-commands.md`, the "Viewers and limits" section.
//!
//! The watching is deliberately **not** something the interface asks for over and over. It
//! is switched on, and from then on the list arrives as a stream (`viewers:update`). Asking
//! again and again for something that changes every few seconds is what SC-009 exists to
//! prevent, and it would double the traffic to the server for nothing.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use super::error::Result;
use super::AppState;
use crate::domain::access_log::Asked;
use crate::domain::geo::Place;
use crate::domain::viewers::{VariantFacts, Viewer};
use crate::server::viewers::{self, ViewerContext, ViewersUpdate, Watch};

/// What the watching of one server holds while it runs.
///
/// One at a time, on purpose. Two servers watched at once would take four standing channels
/// out of the two there are (R-04, T153), and a person looks at one server's viewers
/// anyway — the one whose library is on the screen.
#[derive(Default)]
pub struct ViewersWatch {
    inner: Mutex<Option<Running>>,
}

struct Running {
    server_id: String,
    watch: Watch,
}

impl ViewersWatch {
    /// Which server is being watched, if any.
    pub fn watching(&self) -> Option<String> {
        self.inner
            .lock()
            .ok()
            .and_then(|g| g.as_ref().map(|r| r.server_id.clone()))
    }

    /// The watching of this server as it stands — `None` when it is not being watched, or
    /// when the watch there has given up and has to be started afresh (T664).
    pub fn alive_for(&self, server_id: &str) -> Option<ViewersUpdate> {
        self.inner.lock().ok().and_then(|g| {
            g.as_ref()
                .filter(|r| r.server_id == server_id && r.watch.is_alive())
                .map(|r| r.watch.current())
        })
    }

    /// Where the watching stands, when there is one.
    pub fn status(&self) -> Option<crate::domain::viewers::WatchStatus> {
        self.inner
            .lock()
            .ok()
            .and_then(|g| g.as_ref().map(|r| r.watch.status()))
    }

    /// Who is watching right now, by the server's clock as last read.
    ///
    /// Empty when nothing is being watched, which is a true answer rather than a
    /// failure: nobody is known to be watching.
    pub fn active_now(&self) -> Vec<crate::domain::viewers::Viewer> {
        self.inner
            .lock()
            .ok()
            .and_then(|g| {
                g.as_ref()
                    .map(|r| r.watch.active(time::OffsetDateTime::now_utc()))
            })
            .unwrap_or_default()
    }

    fn replace(&self, running: Option<Running>) {
        if let Ok(mut guard) = self.inner.lock() {
            // The previous one is dropped here, which stops it and gives its two channels
            // back. Doing it in this order matters: starting a second watch before the
            // first has let go would ask for a third and a fourth standing channel, and
            // there are only two.
            *guard = running;
        }
    }

    fn history(&self) -> Vec<Viewer> {
        self.inner
            .lock()
            .ok()
            .and_then(|g| g.as_ref().map(|r| r.watch.history()))
            .unwrap_or_default()
    }

    /// Tell a running watch that the threshold has changed.
    pub fn set_threshold(&self, threshold: time::Duration) {
        if let Ok(guard) = self.inner.lock() {
            if let Some(running) = guard.as_ref() {
                running.watch.set_threshold(threshold);
            }
        }
    }
}

/// What the library and the table of places can say about a viewer.
///
/// A snapshot rather than a live look-up: the library is on the server, and going back to
/// it for every line of the log would mean a round trip per segment — several a second for
/// every viewer at once.
struct LibraryContext {
    /// The served file's name to what is known about it.
    by_file: HashMap<String, VariantFacts>,
    /// A quality set's short name to the medium it belongs to.
    by_slug: HashMap<String, String>,
    /// A quality set's short name to what each of its rungs needs (T666) — read off the
    /// set's description on the server when the watching starts.
    needs: HashMap<String, HashMap<String, u64>>,
    places: Arc<std::sync::RwLock<crate::store::geo::Places>>,
}

impl ViewerContext for LibraryContext {
    fn facts(&self, asked: &Asked) -> VariantFacts {
        match asked {
            Asked::DirectFile { name } => self.by_file.get(name).cloned().unwrap_or_default(),
            Asked::SetDescription { slug, .. }
            | Asked::RungPlaylist { slug, .. }
            | Asked::Segment { slug, .. }
            | Asked::SetInit { slug, .. } => crate::domain::viewers::set_facts(
                asked,
                self.by_slug.get(slug).cloned(),
                // What a rung needs is its BANDWIDTH in the set's description (QA-24B-07:
                // this used to be left empty, and SlowLink could never fire for a viewer of
                // a set). A set whose description could not be read still has none — the
                // medium's average is not what any one rung needs.
                self.needs.get(slug),
            ),
            Asked::Other => VariantFacts::default(),
        }
    }

    fn place(&self, ip: &str) -> Place {
        // Read under a lock rather than copied once at the start: the tables may arrive
        // while a session is already being watched, and a viewer who appeared before them
        // should be placed as soon as they land.
        self.places
            .read()
            .map(|p| p.look_up(ip))
            .unwrap_or_default()
    }
}

impl LibraryContext {
    fn build(
        view: &super::library::LibraryView,
        places: Arc<std::sync::RwLock<crate::store::geo::Places>>,
    ) -> Self {
        let mut by_file = HashMap::new();
        let mut by_slug = HashMap::new();

        for media in &view.media {
            for file in &media.files {
                by_file.insert(
                    file.path.clone(),
                    VariantFacts {
                        media_id: Some(media.id.clone()),
                        variant: Some(file.path.clone()),
                        required_bps: file.bitrate_bps,
                    },
                );
            }
            // The set's prepared rung files (T678): served by their own names too, though
            // nobody is handed them; a viewer on one is on this medium.
            for file in &media.set_files {
                by_file.insert(
                    file.path.clone(),
                    VariantFacts {
                        media_id: Some(media.id.clone()),
                        variant: Some(file.path.clone()),
                        required_bps: file.bitrate_bps,
                    },
                );
            }
            // A ladder is recorded by the path of its description; what a viewer asks for
            // is named by the directory it sits in.
            for ladder in &media.ladders {
                if let Some(slug) = ladder.path.split('/').next() {
                    by_slug.insert(slug.to_owned(), media.id.clone());
                }
            }
            // The short name of the medium itself, for a set that is served under it
            // without being written into the catalogue as a ladder.
            by_slug
                .entry(media.slug.clone())
                .or_insert_with(|| media.id.clone());
        }

        // The files nobody has claimed. A viewer watching one of those is watching
        // something real, and hiding them because the catalogue says nothing would be
        // worse than showing them under their file name.
        for file in &view.unrecognized {
            by_file.entry(file.path.clone()).or_insert(VariantFacts {
                media_id: None,
                variant: Some(file.path.clone()),
                required_bps: file.bitrate_bps,
            });
        }

        Self {
            by_file,
            by_slug,
            needs: HashMap::new(),
            places,
        }
    }

    /// The quality sets on the server, by short name — the ones whose rungs' needs are read.
    fn set_slugs(view: &super::library::LibraryView) -> Vec<String> {
        let mut slugs: Vec<String> = view
            .media
            .iter()
            .flat_map(|m| m.ladders.iter())
            .filter(|l| l.exists_on_server)
            .filter_map(|l| l.path.split('/').next().map(str::to_owned))
            .collect();
        slugs.sort();
        slugs.dedup();
        slugs
    }
}

/// How a lost watching gets a new connection (T664).
///
/// **To the machine the watching began on, as it was confirmed then** — the profile as it
/// was read at the start. Read afresh at every try it would follow an edit of the profile to
/// another machine, and the list of that one's viewers would arrive under this one's name
/// (the lesson of T647). What is read afresh is only whether the profile still exists: a
/// server that was removed is not watched on.
///
/// Refusals are sorted into those that pass and those that do not: an address that does not
/// answer is the network, and it comes back; a changed key, a refused login or a key that
/// cannot be read wait for a person, and asking again every thirty seconds would only fill
/// the server's log with failed logins.
fn reconnect_to(
    state: &AppState,
    profile: crate::domain::server_profile::ServerProfile,
) -> viewers::Reconnect {
    let secrets = state.secrets.clone();
    let db = state.db.clone();
    Arc::new(move || {
        let secrets = secrets.clone();
        let db = db.clone();
        let profile = profile.clone();
        Box::pin(async move {
            match crate::store::profiles::get(&db, &profile.id) {
                Ok(Some(_)) => {}
                Ok(None) => {
                    return Err(viewers::Retry::Permanent(String::from(
                        "the server's profile was removed",
                    )))
                }
                Err(e) => return Err(viewers::Retry::Transient(e.to_string())),
            }
            crate::server::gate::open(
                secrets.as_ref(),
                &profile,
                crate::server::gate::Intent::Read,
            )
            .await
            .map(|opened| opened.conn)
            .map_err(sort_refusal)
        })
    })
}

/// Whether a refusal to connect passes by itself — see [`reconnect_to`].
pub fn sort_refusal(refusal: crate::server::gate::Refusal) -> viewers::Retry {
    use crate::server::gate::Refusal;
    use crate::ssh::SshError;
    let why = refusal.to_string();
    match refusal {
        Refusal::Ssh(
            SshError::Unreachable { .. }
            | SshError::Exec(_)
            | SshError::Protocol(_)
            | SshError::Sftp { .. },
        ) => viewers::Retry::Transient(why),
        _ => viewers::Retry::Permanent(why),
    }
}

pub mod api {
    use super::*;

    /// Start watching a server's viewers.
    ///
    /// Repeating it for the same server is not an error and not a second watch: it is the
    /// ordinary thing to do when a screen is opened again, and starting a second would take
    /// standing channels that do not exist.
    ///
    /// ⚠ **But only over a watch that is alive** (T664, QA-24B-05). The name of the server
    /// used to be enough, and a watch whose connection had died long ago answered "already
    /// watching" for ever — reopening the screen could not bring it back. A watch that has
    /// given up is replaced; one that is running, or getting its connection back, is kept,
    /// and what it has is sent at once so the screen does not wait for the next poll.
    pub async fn viewers_watch_start(state: &AppState, server_id: &str) -> Result<()> {
        if let Some(current) = state.viewers.alive_for(server_id) {
            // Only when there is something to say: before the first list, an empty one would
            // read as "nobody is watching" when nobody has looked yet.
            if current.as_of.is_some()
                || current.watch != crate::domain::viewers::WatchState::Watching
            {
                let _ = state
                    .events
                    .send(super::super::AppEvent::ViewersUpdate(current));
            }
            return Ok(());
        }
        // Whatever was being watched stops first, so that its two channels come back before
        // the new watch asks for its own.
        state.viewers.replace(None);

        let profile = crate::store::profiles::get(&state.db, server_id)?
            .ok_or_else(|| super::super::servers::no_such_server(server_id))?;
        let view = super::super::library::api::library_list(state, server_id, false).await?;
        let settings = crate::store::settings::load(&state.db)?;
        let mut context = LibraryContext::build(&view, state.places.clone());

        // Watching only. A server that is somebody else's still shows who is pulling
        // from it — and that is exactly the sort of thing its owner would want to see.
        let conn = crate::server::gate::open(
            state.secrets.as_ref(),
            &profile,
            crate::server::gate::Intent::Read,
        )
        .await?
        .conn;

        // What each rung of each set needs (T666), read once here, as the library itself is.
        // A set rebuilt while the watching runs (which is asked about when anybody is
        // watching — T571) keeps its old figures until the screen is opened again.
        context.needs =
            viewers::rung_needs(&conn, &profile.video_dir, &LibraryContext::set_slugs(&view)).await;
        let context = Arc::new(context);

        let (tx, mut updates) = tokio::sync::mpsc::channel(64);
        let watch = viewers::start_reconnecting(
            conn,
            Some(reconnect_to(state, profile.clone())),
            server_id.to_owned(),
            context,
            settings.activity_threshold(),
            tx,
        )
        .await?;

        // The updates are carried outwards on their own task: whoever asked for the
        // watching gets an answer at once, and the list arrives as it changes.
        let events = state.events.clone();
        tokio::spawn(async move {
            while let Some(update) = updates.recv().await {
                if events
                    .send(super::super::AppEvent::ViewersUpdate(update))
                    .is_err()
                {
                    // Nobody is listening. The watch itself is stopped by whoever holds it,
                    // not from here — the list may still be wanted by `viewers_history`.
                    break;
                }
            }
        });

        state.viewers.replace(Some(Running {
            server_id: server_id.to_owned(),
            watch,
        }));
        Ok(())
    }

    /// Stop watching. Quiet when nothing was being watched: the interface closes a screen
    /// it may never have opened.
    pub fn viewers_watch_stop(state: &AppState) {
        state.viewers.replace(None);
    }

    /// Those who watched earlier in this session (FR-055).
    ///
    /// Kept only while the application runs. Nothing about a viewer is written down: the
    /// data model says so, and an address is somebody's whereabouts, not our record to
    /// keep.
    pub fn viewers_history(state: &AppState) -> Vec<Viewer> {
        state.viewers.history()
    }
}

pub mod ipc {
    use super::*;
    use tauri::State;

    #[tauri::command]
    pub async fn viewers_watch_start(state: State<'_, AppState>, server_id: String) -> Result<()> {
        api::viewers_watch_start(&state, &server_id).await
    }

    #[tauri::command]
    pub async fn viewers_watch_stop(state: State<'_, AppState>) -> Result<()> {
        api::viewers_watch_stop(&state);
        Ok(())
    }

    #[tauri::command]
    pub async fn viewers_history(state: State<'_, AppState>) -> Result<Vec<Viewer>> {
        Ok(api::viewers_history(&state))
    }
}

/// Re-exported so the event bridge can name what it carries.
pub type Update = ViewersUpdate;
