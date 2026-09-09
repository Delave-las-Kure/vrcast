//! T162 — keeping the tables of places up to date.
//!
//! Two ways in, and both matter. On start the application looks whether the month has
//! turned and fetches quietly in the background — FR-112 allows it to get what it needs
//! itself, but not to ask the person for anything. And a person who wants it now can say
//! so, because "it will sort itself out eventually" is a poor answer to somebody looking at
//! a screen full of "not determined".

use std::sync::Arc;

use super::error::Result;
use super::AppState;
use crate::store::geo;

/// What is known about the tables right now.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GeoStatus {
    /// Which month's tables are in place. Absent means there are none yet.
    pub month: Option<String>,
    /// Whether anything can be answered at all.
    pub ready: bool,
    /// Whether a newer month is out.
    pub stale: bool,
}

fn this_month() -> (i32, u8) {
    let now = time::OffsetDateTime::now_utc();
    (now.year(), now.month() as u8)
}

pub mod api {
    use super::*;

    pub fn geo_status(state: &AppState) -> GeoStatus {
        let (year, month) = this_month();
        let wanted = geo::month_name(year, month);
        let Some(dir) = geo::dir() else {
            return GeoStatus {
                month: None,
                ready: false,
                stale: true,
            };
        };
        GeoStatus {
            month: geo::month_on_disk(&dir),
            ready: state.places.read().map(|p| !p.is_empty()).unwrap_or(false),
            stale: geo::needs_fetching(&dir, &wanted),
        }
    }

    /// Fetch the tables and put them to work.
    ///
    /// The newly opened tables replace the old under the same lock, so a session already
    /// being watched starts placing its viewers without waiting for a restart.
    ///
    /// Serialized against `refresh_in_background` through `state.geo_fetch` (T569): both
    /// call `geo::fetch` into the same temp files, and running at once could corrupt them.
    ///
    /// **A small, deliberate change in behaviour.** Once the lock is held, this always
    /// re-checks whether the tables on disk already match the wanted month before calling
    /// `geo::fetch` again — not only when this call had to wait for the lock. `tokio::sync::
    /// Mutex` gives no cheap way to tell "I waited" from "I walked straight in", and always
    /// re-checking is simpler than threading that distinction through. The user-visible
    /// effect: pressing "Fetch" when the tables are already current now sometimes does not
    /// touch the network at all, where before it always did. It still fetches unconditionally
    /// whenever the tables are *not* already current — which is every case that matters to
    /// someone pressing the button expecting it to do something.
    pub async fn geo_update(state: &AppState) -> Result<GeoStatus> {
        let Some(dir) = geo::dir() else {
            return Ok(geo_status(state));
        };
        let (year, month) = this_month();
        let wanted = geo::month_name(year, month);
        let _guard = state.geo_fetch.lock().await;

        if geo::month_on_disk(&dir).as_deref() == Some(wanted.as_str()) {
            // Someone else — the background refresh, most likely — brought the tables up
            // to date while we were waiting for the lock (or already had, before we even
            // asked for it). `state.places` is shared behind an `Arc`, so whoever fetched
            // it has already put it to work; nothing left to do here.
            return Ok(geo_status(state));
        }

        match geo::fetch(&dir, year, month).await {
            Ok(taken) => {
                let opened = geo::Places::open(&dir);
                if let Ok(mut places) = state.places.write() {
                    *places = opened;
                }
                tracing::info!(month = %taken, "the tables of places were brought up to date");
            }
            Err(e) => {
                // Not an error to the person. Without the tables everything works and every
                // viewer is "not determined"; refusing over it would be out of proportion.
                tracing::info!(error = %e, "the tables of places could not be fetched");
            }
        }
        Ok(geo_status(state))
    }

    /// Bring them up to date in the background if the month has turned.
    ///
    /// Started once, at start-up. Nothing waits for it and nothing is shown while it runs:
    /// seventy megabytes take a few seconds on a good connection and a long time on a bad
    /// one, and neither should hold up a person who came to upload a film.
    ///
    /// Serialized against `geo_update` through `state.geo_fetch` (T569) — see there for why.
    pub fn refresh_in_background(state: &AppState) {
        let (year, month) = this_month();
        let Some(dir) = geo::dir() else { return };
        let wanted = geo::month_name(year, month);
        if !geo::needs_fetching(&dir, &wanted) {
            return;
        }
        let places = Arc::clone(&state.places);
        let geo_fetch = Arc::clone(&state.geo_fetch);
        tauri::async_runtime::spawn(async move {
            let _guard = geo_fetch.lock().await;
            // Re-checked now the lock is held: "Fetch" may have already brought the same
            // month in while this task waited its turn, and fetching it a second time
            // would only waste the person's bandwidth for no gain.
            if geo::month_on_disk(&dir).as_deref() == Some(wanted.as_str()) {
                return;
            }
            if geo::fetch(&dir, year, month).await.is_ok() {
                let opened = geo::Places::open(&dir);
                if let Ok(mut guard) = places.write() {
                    *guard = opened;
                }
            }
        });
    }
}

pub mod ipc {
    use super::*;
    use tauri::State;

    #[tauri::command]
    pub async fn geo_status(state: State<'_, AppState>) -> Result<GeoStatus> {
        Ok(api::geo_status(&state))
    }

    #[tauri::command]
    pub async fn geo_update(state: State<'_, AppState>) -> Result<GeoStatus> {
        api::geo_update(&state).await
    }
}
