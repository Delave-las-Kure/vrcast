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
    ///
    /// The same holds for `concurrent_heavy_tasks` (T546): every `TaskEngine` handle shares
    /// one lock around its lane limits (see `tasks::engine::TaskEngine::set_limits`), so
    /// telling this one is enough — no restart, and nothing else in `AppState` needs to
    /// change. `Light` is left alone: it was never governed by this setting.
    pub fn settings_set(state: &AppState, settings: &Settings) -> Result<Settings> {
        let saved = crate::store::settings::save(&state.db, settings)?;
        state.viewers.set_threshold(saved.activity_threshold());
        state.tasks.set_limits(crate::tasks::state::LaneLimits {
            compute: saved.concurrent_heavy_tasks as usize,
            network: saved.concurrent_heavy_tasks as usize,
            light: crate::tasks::state::LaneLimits::default().light,
        });
        Ok(saved)
    }

    /// What is still lying in a working folder (T453).
    ///
    /// Asked after the path is changed, about the **old** one. Working files are swept after
    /// a variant is sent, so this is nothing nearly always — and when it is not, it is one
    /// and a half to two gigabytes left by a build that was killed, under a path the
    /// application will never look at again.
    ///
    /// A reading, not an act. Moving gigabytes between disks takes minutes and would happen
    /// inside a click, with nothing to watch and no way to stop it; deleting somebody's files
    /// because they changed a setting is worse. What is owed here is the fact.
    pub fn work_dir_leftovers(path: &str) -> Result<crate::domain::work_dir::Leftovers> {
        Ok(crate::domain::work_dir::leftovers_in(std::path::Path::new(
            path,
        )))
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

    #[tauri::command]
    pub async fn work_dir_leftovers(path: String) -> Result<crate::domain::work_dir::Leftovers> {
        api::work_dir_leftovers(&path)
    }
}
