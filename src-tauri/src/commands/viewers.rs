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
    places: Arc<std::sync::RwLock<crate::store::geo::Places>>,
}

impl ViewerContext for LibraryContext {
    fn facts(&self, asked: &Asked) -> VariantFacts {
        match asked {
            Asked::DirectFile { name } => self.by_file.get(name).cloned().unwrap_or_default(),
            Asked::SetDescription { slug, .. }
            | Asked::RungPlaylist { slug, .. }
            | Asked::Segment { slug, .. } => VariantFacts {
                media_id: self.by_slug.get(slug).cloned(),
                variant: asked.rung().map(str::to_owned),
                // What a rung needs is written in the description of the quality set, and
                // reading that is Phase 5's work (T186). Until then a viewer of a set is
                // shown with everything except the speed they ought to be getting, and
                // SlowLink cannot fire for them. Left honestly empty rather than filled
                // with the medium's average, which is not what any one rung needs.
                required_bps: None,
            },
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
            // A ladder is recorded by the path of its description; what a viewer asks for
            // is named by the directory it sits in.
            for ladder in &media.ladders {
                if let Some(slug) = ladder.split('/').next() {
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
            places,
        }
    }
}

pub mod api {
    use super::*;

    /// Start watching a server's viewers.
    ///
    /// Repeating it for the same server is not an error and not a second watch: it is the
    /// ordinary thing to do when a screen is opened again, and starting a second would take
    /// standing channels that do not exist.
    pub async fn viewers_watch_start(state: &AppState, server_id: &str) -> Result<()> {
        if state.viewers.watching().as_deref() == Some(server_id) {
            return Ok(());
        }
        // Whatever was being watched stops first, so that its two channels come back before
        // the new watch asks for its own.
        state.viewers.replace(None);

        let profile = crate::store::profiles::get(&state.db, server_id)?
            .ok_or_else(|| super::super::servers::no_such_server(server_id))?;
        let view = super::super::library::api::library_list(state, server_id, false).await?;
        let settings = crate::store::settings::load(&state.db)?;
        let context = Arc::new(LibraryContext::build(&view, state.places.clone()));

        let conn = crate::server::connect(state.secrets.as_ref(), &profile).await?;

        let (tx, mut updates) = tokio::sync::mpsc::channel(64);
        let watch = viewers::start(
            conn,
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
