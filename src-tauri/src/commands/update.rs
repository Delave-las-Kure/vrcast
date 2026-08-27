//! T351, T352 — is there a newer version, and putting it on (FR-113).
//!
//! **Asked for, never volunteered.** The check runs when a person presses the button and at no
//! other time. An application that reaches for the network at startup does so on the machine of
//! somebody working on a train, and on the machine of somebody who would rather it went nowhere
//! at all; neither of them asked.
//!
//! **How this copy was packaged decides what updating costs**, and the packaging is not guessed:
//! the bundler writes it into the binary, and `bundle_type()` reads it back. A copy from a
//! `.deb` updates through `dpkg`, which needs root, so the system will ask for a password
//! before anything happens — worth saying beforehand rather than surprising somebody halfway.
//! An AppImage rewrites itself in place with nothing to ask. A Windows copy is **killed by its
//! own installer** the moment installation starts, which is why the screen shows the same list
//! of running tasks that closing the application shows.
//!
//! **A build with no update settings says so.** The plugin is only registered when
//! `plugins.updater` is in the configuration — see `lib.rs`, and the reason there — so on a
//! build without it the check has nowhere to look and reports exactly that. Silence would read
//! as "you are up to date", which is a different statement and, here, a false one.

use serde::{Deserialize, Serialize};
use tauri::utils::config::BundleType;
use tauri::utils::platform::bundle_type;

use super::error::{AppError, ErrorCode, Result};

/// How the running copy was packaged.
///
/// Not a matter of taste: it decides where the update comes from, whether a password is asked
/// for, and whether the application survives the installation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstalledAs {
    /// The Windows installer. It replaces the files, and it stops the running application to
    /// do it.
    Windows,
    /// A file that rewrites itself where it lies. Nothing to ask, nobody to ask.
    AppImage,
    /// A system package: updating goes through `dpkg` as root, so the system asks first.
    Deb,
    /// A system package: updating goes through `rpm` as root, so the system asks first.
    Rpm,
    /// Not packaged at all — a binary run out of a build directory. There is nothing here for
    /// an installer to replace.
    Unpackaged,
}

impl InstalledAs {
    fn here() -> Self {
        match bundle_type() {
            Some(BundleType::Deb) => Self::Deb,
            Some(BundleType::Rpm) => Self::Rpm,
            Some(BundleType::AppImage) => Self::AppImage,
            Some(BundleType::Msi) | Some(BundleType::Nsis) => Self::Windows,
            // `App` is macOS, which this application does not ship for; anything else, and
            // the absent case, is a build somebody is running from the source tree.
            _ => Self::Unpackaged,
        }
    }
}

/// What the check found.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Found {
    /// This build carries no update settings, so nothing was asked of anybody.
    NotConfigured,
    /// The address answered, and it has nothing newer.
    UpToDate,
    /// There is a newer version.
    Available {
        version: String,
        /// What is in it, as written in the release. Absent is ordinary: an update with no
        /// notes is still an update, and inventing a description would be worse.
        notes: Option<String>,
        /// The day it was published, if the release says.
        date: Option<String>,
    },
}

/// Where this copy stands, answerable without asking anybody anything.
///
/// **Separate from the check on purpose.** The screen needs the version and the packaging the
/// moment it opens; it must not reach for the network to get them. Two commands make that a
/// property of the code rather than a promise in a comment — the one that opens the screen
/// cannot touch the network, because it has no way to.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateStanding {
    /// The version running right now.
    pub current: String,
    pub installed_as: InstalledAs,
    /// Whether this build has anywhere to look at all.
    pub configured: bool,
}

/// Whether this build has anywhere to look.
///
/// The same question `lib.rs` asks before registering the plugin, asked the same way. Two
/// places deciding this differently would mean a screen that offers to check and a runtime with
/// nothing behind it.
fn configured<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> bool {
    app.config().plugins.0.contains_key("updater")
}

pub mod api {
    use super::*;
    use tauri_plugin_updater::UpdaterExt;

    /// The version, the packaging, and whether there is anywhere to look. Touches nothing
    /// outside this machine — see `UpdateStanding` for why that is a separate command.
    pub fn update_standing<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> UpdateStanding {
        UpdateStanding {
            current: app.package_info().version.to_string(),
            installed_as: InstalledAs::here(),
            configured: configured(app),
        }
    }

    /// Ask the address whether there is a newer version. Only ever from a person's press.
    pub async fn update_check<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> Result<Found> {
        if !configured(app) {
            return Ok(Found::NotConfigured);
        }

        let updater = app
            .updater()
            .map_err(|e| AppError::new(ErrorCode::UpdateCheckFailed).with_cause(e))?;

        match updater
            .check()
            .await
            .map_err(|e| AppError::new(ErrorCode::UpdateCheckFailed).with_cause(e))?
        {
            Some(update) => Ok(Found::Available {
                version: update.version.clone(),
                notes: update.body.clone(),
                // The day, not the second: this is read by a person deciding whether to
                // bother, and a timestamp to the nanosecond helps nobody decide anything.
                date: update.date.map(|d| d.date().to_string()),
            }),
            None => Ok(Found::UpToDate),
        }
    }

    /// Fetch the newer version and put it on.
    ///
    /// **On Windows this function does not return.** The installer stops the application as its
    /// first act — the documentation is plain about it — so anything meant to happen afterwards
    /// has to have happened before. That is why the screen shows the running tasks first, and
    /// why the answer to "what about my four-hour encode" is given while there is still
    /// somebody to give it to.
    ///
    /// The check is made again here rather than carried over from the screen. It costs one
    /// request and removes a whole class of mistake: installing what was true five minutes ago.
    pub async fn update_install<R: tauri::Runtime>(
        app: &tauri::AppHandle<R>,
        confirmed: bool,
    ) -> Result<()> {
        // Same rule as everywhere else that cannot be undone by pressing again: no
        // confirmation, no action, and the refusal comes before anything is fetched.
        if !confirmed {
            return Err(AppError::new(ErrorCode::ConfirmationRequired));
        }
        if !configured(app) {
            return Err(AppError::new(ErrorCode::UpdateCheckFailed));
        }

        let updater = app
            .updater()
            .map_err(|e| AppError::new(ErrorCode::UpdateInstallFailed).with_cause(e))?;
        let update = updater
            .check()
            .await
            .map_err(|e| AppError::new(ErrorCode::UpdateCheckFailed).with_cause(e))?
            .ok_or_else(|| AppError::new(ErrorCode::UpdateInstallFailed))?;

        update
            .download_and_install(|_downloaded, _total| {}, || {})
            .await
            .map_err(|e| AppError::new(ErrorCode::UpdateInstallFailed).with_cause(e))?;

        Ok(())
    }
}

pub mod ipc {
    use super::*;

    #[tauri::command]
    pub async fn update_standing(app: tauri::AppHandle) -> Result<UpdateStanding> {
        Ok(api::update_standing(&app))
    }

    #[tauri::command]
    pub async fn update_check(app: tauri::AppHandle) -> Result<Found> {
        api::update_check(&app).await
    }

    #[tauri::command]
    pub async fn update_install(app: tauri::AppHandle, confirmed: bool) -> Result<()> {
        api::update_install(&app, confirmed).await
    }
}
