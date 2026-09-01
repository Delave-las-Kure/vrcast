//! T395 — the tray icon, and the words on its menu.
//!
//! **The words come from the interface** (FR-105, FR-106). The tray is the one place in the
//! core that puts text on a screen, and it holds none of its own: a menu entry has to say
//! something, the nearest string is right there, and `the_tray_puts_no_words_of_its_own_on_the_screen`
//! is the check that keeps it from being written. The interface hands them over at start-up
//! and again whenever the person changes language.
//!
//! **Called rather than built at start-up** for the same reason: at start-up nobody has yet
//! said which language it is. Building the menu then would mean the core choosing, which is
//! the thing it must not do.

use super::error::Result;

pub mod api {
    use super::*;

    /// Whether this machine can show a tray icon at all.
    pub fn tray_state() -> Result<crate::tray::TrayState> {
        Ok(crate::tray::probe())
    }
}

pub mod ipc {
    use super::*;
    use tauri::AppHandle;

    #[tauri::command]
    pub async fn tray_state() -> Result<crate::tray::TrayState> {
        api::tray_state()
    }

    /// Put the icon up, or change what its menu says.
    ///
    /// Quiet where there is no tray: `install` does nothing without a window icon, and
    /// `probe` has already told the interface whether there is anywhere to minimise to. A
    /// failure here is not a reason to stop the application — the worst of it is that a
    /// person closes the window with the button instead of the menu.
    #[tauri::command]
    pub async fn tray_labels(app: AppHandle, labels: crate::tray::Labels) -> Result<()> {
        if let Err(e) = crate::tray::install(&app, &labels) {
            tracing::warn!(error = %e, "the tray icon would not go up");
        }
        Ok(())
    }
}
