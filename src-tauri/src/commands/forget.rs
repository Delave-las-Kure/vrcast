//! T356, T357, T358 — removing everything the application keeps about a person (FR-114).
//!
//! **Why the application does this and not the uninstaller.** Three formats are handed out and
//! only one of them can ask a question at removal time: the Windows uninstaller has its
//! checkbox, a `.deb` runs its removal script with no way to prompt, and an AppImage is not
//! installed at all — it is a file somebody deleted. The application is the one place present
//! in all three.
//!
//! **And it is the only place that can reach the secrets.** They live in the operating
//! system's own store (constitution, principle IV), not in the data directory, so nothing that
//! deletes directories touches them. Left behind they are entries belonging to a program that
//! no longer exists — and after the removal there is nobody to clear them.
//!
//! **What this deliberately does not remove: the webview's own cache.** It sits under the
//! identifier — under LOCALAPPDATA, 276 files on the machine this was written
//! on — and it holds no profiles and no secrets, only what a browser engine keeps. On Windows
//! the uninstaller's checkbox already clears it, and on the others it is a cache the system
//! may clear itself. Named here rather than left unsaid: an omission with no reason beside it
//! reads as an oversight, and the next person to look adds it without knowing why it was out.
//!
//! **The key the application made for itself is a special case, and a dangerous one.** A
//! server deployed by this application has password logins turned off (the `ssh-hardening`
//! step). The private half of the key that replaced them is in the OS store and nowhere else.
//! Erasing it without keeping a copy means losing the server for good — the way back is the
//! hosting provider's console and a reinstall. So it is counted separately, said out loud, and
//! offered for saving before anything is erased.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::domain::server_profile::AuthKind;
use crate::store::secrets::SecretRef;

use super::error::{AppError, ErrorCode, Result};
use super::AppState;

/// What removal would take, named piece by piece.
///
/// **A list, not a promise.** "Delete my data" without one is read differently by everybody
/// who reads it, and the person deciding is the one who cannot check afterwards.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhatWouldGo {
    /// The directory holding the database, the library cache and the place tables.
    pub data_dir: Option<String>,
    /// How much is in it, in bytes. The place tables alone are over a hundred megabytes, and
    /// somebody clearing space deserves to know that is what they are clearing.
    pub bytes: u64,
    /// How many server profiles, with their names.
    pub servers: Vec<String>,
    /// How many secrets are in the operating system's store.
    pub secrets: usize,
    /// **Servers that would become unreachable.** Deployed by this application, with password
    /// logins off, and the only key for them is the one about to be erased.
    pub locked_out: Vec<String>,
}

/// The result of actually removing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhatWent {
    pub data_dir_removed: bool,
    pub secrets_removed: usize,
    /// Secrets the store refused to give up. Reported rather than swallowed: a person told
    /// "everything is gone" while entries remain has been told something false.
    pub secrets_left: Vec<String>,
}

/// Where this run keeps its things — **from the state, never from the environment**.
///
/// The difference is not academic. The first version of this worked the path out from
/// `ProjectDirs` directly, which meant every caller pointed at the real directory whatever
/// state it was given; a contract test running on an in-memory database duly deleted the
/// developer's own profiles and both place tables. Taken from the state, a run that was not
/// given a directory removes nothing, and that is decided at construction rather than by
/// whoever calls this.
fn data_dir(state: &AppState) -> Option<PathBuf> {
    state.data_dir.clone()
}

/// Everything under a directory, in bytes. Missing or unreadable counts as nothing: this is
/// shown to a person, and refusing to answer at all would be worse than answering roughly.
fn weigh(dir: &std::path::Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .map(|e| match e.file_type() {
            Ok(t) if t.is_dir() => weigh(&e.path()),
            Ok(_) => e.metadata().map(|m| m.len()).unwrap_or(0),
            Err(_) => 0,
        })
        .sum()
}

pub mod api {
    use super::*;

    /// What would go, so that the person can look before deciding (FR-114).
    pub fn forget_preview(state: &AppState) -> Result<WhatWouldGo> {
        let profiles = crate::store::profiles::list(&state.db)
            .map_err(|e| AppError::new(ErrorCode::StorageFailed).with_cause(e))?;

        // A server is a lock-out risk when the application made the key itself: then the
        // private half exists only in the OS store, and the server refuses passwords.
        let locked_out = profiles
            .iter()
            .filter(|p| p.auth_kind == AuthKind::ManagedKey)
            .map(|p| p.name.clone())
            .collect();

        let dir = data_dir(state);
        Ok(WhatWouldGo {
            bytes: dir.as_deref().map(weigh).unwrap_or(0),
            data_dir: dir.map(|d| d.display().to_string()),
            servers: profiles.iter().map(|p| p.name.clone()).collect(),
            secrets: profiles.len(),
            locked_out,
        })
    }

    /// Remove it all.
    ///
    /// **Secrets first, and that order is the point.** They can only be reached through the
    /// profiles, and the profiles live in the database that is about to be deleted. Delete the
    /// directory first and the entries in the operating system's store become unreachable
    /// orphans — nothing left knows their names.
    pub fn forget_everything(state: &AppState, confirmed: bool) -> Result<WhatWent> {
        if !confirmed {
            return Err(AppError::new(ErrorCode::ConfirmationRequired));
        }

        let profiles = crate::store::profiles::list(&state.db)
            .map_err(|e| AppError::new(ErrorCode::StorageFailed).with_cause(e))?;

        let mut removed = 0usize;
        let mut left = Vec::new();
        for profile in &profiles {
            let reference = SecretRef::from_stored(profile.secret_ref.clone());
            match state.secrets.delete(&reference) {
                Ok(()) => removed += 1,
                // Named, not counted: which server's secret stayed behind is what a person
                // needs to go and clear it by hand.
                Err(_) => left.push(profile.name.clone()),
            }
        }

        let data_dir_removed = match data_dir(state) {
            Some(dir) if dir.exists() => std::fs::remove_dir_all(&dir).is_ok(),
            // Nothing there is not a failure: a person may have cleared it already, and
            // reporting "could not remove" for an absence would send them looking for it.
            _ => true,
        };

        Ok(WhatWent {
            data_dir_removed,
            secrets_removed: removed,
            secrets_left: left,
        })
    }
}

pub mod ipc {
    use super::*;
    use tauri::State;

    #[tauri::command]
    pub async fn forget_preview(state: State<'_, AppState>) -> Result<WhatWouldGo> {
        api::forget_preview(&state)
    }

    #[tauri::command]
    pub async fn forget_everything(
        state: State<'_, AppState>,
        confirmed: bool,
    ) -> Result<WhatWent> {
        api::forget_everything(&state, confirmed)
    }
}
