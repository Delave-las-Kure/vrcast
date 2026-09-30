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
//! the uninstaller's checkbox already clears it (`uninstall.nsh`), on `.deb` a purge does not
//! touch it either (`scripts/deb-postrm.sh` only clears the data directory this module also
//! clears, not the cache), and on the others it is a cache the system may clear itself. Named
//! here rather than left unsaid: an omission with no reason beside it reads as an oversight,
//! and the next person to look adds it without knowing why it was out.
//!
//! **This module's directory removal is what a `.deb` purge relies on too, indirectly — not
//! by calling it.** `scripts/deb-postrm.sh` (T497) removes the same on-disk data directory
//! this deletes, from dpkg's `postrm purge` hook, because a package's maintainer scripts
//! cannot call into the application (it is already gone by the time they run) and cannot
//! reach the OS secret store either — the same secrets gap this module's own doc-comment
//! above describes for the general case. A `.deb` purge, like the Windows uninstaller's
//! checkbox, clears the data directory and leaves the keyring entries behind.
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

/// ⚠ T648 (QA-23 №2) — **what the person was shown when they agreed**, handed back with the
/// removal: the profiles and the servers that would be lost for good, from the
/// [`WhatWouldGo`] on their screen.
///
/// The finding: the screen read the list once, when it opened. A deployment still going then
/// had not made its key yet, so nothing was named as lost; it finished, the profile turned to
/// `managed_key`, the button came back on the old agreement — and the only key to that server
/// went without a word of warning. The screen now reads the list again, but a screen is not
/// what protects: the removal compares this with what would go *now*, under the same locks it
/// removes under, and refuses as `FORGET_PREVIEW_STALE` when they differ. Order does not
/// matter, the names do: an agreement to lose server A is not one to lose server B.
///
/// The size of the directory and the number of secrets are deliberately not part of it: the
/// directory grows while the application is open (the library cache, the logs), and a refusal
/// over a few kilobytes would teach a person to click through refusals. The number of secrets
/// is the number of profiles, which is compared by name.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForgetSeen {
    /// `WhatWouldGo::servers` as shown.
    pub servers: Vec<String>,
    /// `WhatWouldGo::locked_out` as shown — the warning the person read, or did not get.
    pub locked_out: Vec<String>,
}

impl ForgetSeen {
    /// The same list, whatever the order it was shown in.
    fn normalised(&self) -> (Vec<String>, Vec<String>) {
        let mut servers = self.servers.clone();
        let mut locked_out = self.locked_out.clone();
        servers.sort();
        locked_out.sort();
        (servers, locked_out)
    }
}

impl From<&WhatWouldGo> for ForgetSeen {
    fn from(would: &WhatWouldGo) -> Self {
        Self {
            servers: would.servers.clone(),
            locked_out: would.locked_out.clone(),
        }
    }
}

/// What the profiles say would go: the names, and those whose only key is in here.
///
/// One function for the preview and for the removal's own check (T648), so the two cannot
/// come to disagree about who counts as locked out.
fn seen_in(profiles: &[crate::domain::server_profile::ServerProfile]) -> ForgetSeen {
    ForgetSeen {
        servers: profiles.iter().map(|p| p.name.clone()).collect(),
        // A server is a lock-out risk when the application made the key itself: then the
        // private half exists only in the OS store, and the server refuses passwords.
        locked_out: profiles
            .iter()
            .filter(|p| p.auth_kind == AuthKind::ManagedKey)
            .map(|p| p.name.clone())
            .collect(),
    }
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
        let seen = seen_in(&profiles);

        let dir = data_dir(state);
        Ok(WhatWouldGo {
            bytes: dir.as_deref().map(weigh).unwrap_or(0),
            data_dir: dir.map(|d| d.display().to_string()),
            servers: seen.servers,
            secrets: profiles.len(),
            locked_out: seen.locked_out,
        })
    }

    /// Remove it all.
    ///
    /// **Secrets first, and that order is the point.** They can only be reached through the
    /// profiles, and the profiles live in the database that is about to be deleted. Delete the
    /// directory first and the entries in the operating system's store become unreachable
    /// orphans — nothing left knows their names.
    ///
    /// ⚠ **T643 (QA-22 №3) — not while anything is at work, and nothing starts meanwhile.** A
    /// deployment's `key_keeper` writes the key it made into the store; one already past its
    /// look re-created the secret after this reported it gone. A lock around the removal
    /// alone would not do — the task would write once it was let go — so, by the owner's
    /// decision of 2026-09-30:
    ///
    /// - refused as `FORGET_TASKS_RUNNING` while any task is alive in the engine (queued,
    ///   running, or paused with its work held) or a command holds a claim for one it is about
    ///   to create; nothing is touched. [`TaskEngine::close_for_forgetting`] makes the look and
    ///   the closing one step;
    /// - while this runs, the engine is closed: a new task, a raised one or a claim is refused
    ///   as `FORGET_IN_PROGRESS`, and so is a second removal. It opens again when this returns,
    ///   whichever way;
    /// - the secrets go under [`Db::sign_in_lock`], the lock the other writers of a secret
    ///   (`server_update`, the deployment's key keeping) hold around their whole change — so
    ///   one of them already inside it is waited for rather than overtaken.
    ///
    /// [`TaskEngine::close_for_forgetting`]: crate::tasks::engine::TaskEngine::close_for_forgetting
    /// [`Db::sign_in_lock`]: crate::store::db::Db::sign_in_lock
    ///
    /// ⚠ **T648 (QA-23 №2) — only what the person agreed to.** `seen` is the list on their
    /// screen ([`ForgetSeen`]). It is compared with the profiles read *here*, after the engine
    /// is closed and under the sign-in lock — so nothing can change them between the check and
    /// the removal — and a difference in the names or in who would be locked out is refused as
    /// `FORGET_PREVIEW_STALE`, nothing touched. The order of the refusals: no `confirmed`
    /// first, then a task alive (its end may change the list anyway), then a stale list.
    pub fn forget_everything(
        state: &AppState,
        confirmed: bool,
        seen: &ForgetSeen,
    ) -> Result<WhatWent> {
        if !confirmed {
            return Err(AppError::new(ErrorCode::ConfirmationRequired));
        }

        let _closed = state
            .tasks
            .close_for_forgetting()
            .map_err(|busy| match busy {
                Some(working) => AppError::new(ErrorCode::ForgetTasksRunning).with_cause(working),
                None => AppError::new(ErrorCode::ForgetInProgress),
            })?;
        let _sign_in = state.db.sign_in_lock();

        let profiles = crate::store::profiles::list(&state.db)
            .map_err(|e| AppError::new(ErrorCode::StorageFailed).with_cause(e))?;

        let now = seen_in(&profiles);
        if now.normalised() != seen.normalised() {
            return Err(
                AppError::new(ErrorCode::ForgetPreviewStale).with_cause(format!(
                    "agreed to servers {:?}, locked out {:?}; now servers {:?}, locked out {:?}",
                    seen.servers, seen.locked_out, now.servers, now.locked_out
                )),
            );
        }
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
        seen: ForgetSeen,
    ) -> Result<WhatWent> {
        api::forget_everything(&state, confirmed, &seen)
    }
}
