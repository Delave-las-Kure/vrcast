//! T173 — the settings commands.
//!
//! The contract: `contracts/ipc-commands.md`, the "Settings" section.

use super::error::Result;
use super::AppState;
use crate::store::settings::Settings;

pub mod api {
    use super::*;

    pub fn settings_get(state: &AppState) -> Result<Settings> {
        Ok(crate::store::settings::load(&state.db)?)
    }

    /// Change the settings.
    ///
    /// A running watch is told about the new threshold at once rather than at the next
    /// start. Otherwise a person moves the slider, nothing happens, and they conclude the
    /// setting does nothing — which, until they restarted, would be true.
    pub fn settings_set(state: &AppState, settings: &Settings) -> Result<Settings> {
        let saved = crate::store::settings::save(&state.db, settings)?;
        state.viewers.set_threshold(saved.activity_threshold());
        Ok(saved)
    }
}

pub mod ipc {
    use super::*;
    use tauri::State;

    #[tauri::command]
    pub async fn settings_get(state: State<'_, AppState>) -> Result<Settings> {
        api::settings_get(&state)
    }

    #[tauri::command]
    pub async fn settings_set(state: State<'_, AppState>, settings: Settings) -> Result<Settings> {
        api::settings_set(&state, &settings)
    }
}
