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
    pub async fn geo_update(state: &AppState) -> Result<GeoStatus> {
        let Some(dir) = geo::dir() else {
            return Ok(geo_status(state));
        };
        let (year, month) = this_month();
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
    pub fn refresh_in_background(state: &AppState) {
        let (year, month) = this_month();
        let Some(dir) = geo::dir() else { return };
        if !geo::needs_fetching(&dir, &geo::month_name(year, month)) {
            return;
        }
        let places = Arc::clone(&state.places);
        tauri::async_runtime::spawn(async move {
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
