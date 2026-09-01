//! T393, T394 — whether there is anywhere to minimise to, and what the close button does.
//!
//! **The decision this module exists to get right.** Minimising to the tray instead of
//! closing is a good default only where a tray exists. On Windows one always does. On Linux
//! it depends on the desktop: the icon is drawn by an AppIndicator implementation, and on a
//! session without one the window would vanish with no way to bring it back — the worst
//! outcome available, because the application is still running, still holding encodes, and
//! there is nothing on screen to say so.
//!
//! So the close button asks first, and where there is nowhere to go it closes as it always
//! did.
//!
//! **What can and cannot be known.** Whether the library loads can be known. Whether a panel
//! is actually showing the icon **cannot** — there is no call that answers it, and
//! `TrayIconEvent` is documented as unsupported on Linux, so a click cannot be waited for
//! either (R-35). That is why the setting in T399 exists: the probe covers the case that can
//! be detected, and the person covers the rest.
//!
//! The loader is a parameter so that all of this can be checked without the library being
//! installed — on this project's own Windows machine, for instance, where none of the three
//! names exists at all.

/// Whether the system can show a tray icon.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrayState {
    /// There is somewhere to minimise to.
    Installed,
    /// There is not. The window must not be hidden.
    Unavailable,
}

/// What pressing the close button does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseAction {
    /// Hide the window; the application goes on working.
    Hide,
    /// End the application, as it always did.
    Exit,
}

/// What the close button must do, given what the system can show.
///
/// Pure and one line, and separate all the same: it is the whole of the decision, and the
/// only part of it that can be checked without a desktop session.
pub fn close_action(state: TrayState) -> CloseAction {
    match state {
        TrayState::Installed => CloseAction::Hide,
        TrayState::Unavailable => CloseAction::Exit,
    }
}

/// The libraries that give a tray icon on Linux, in the order they are tried.
///
/// **Both, and in this order.** Ayatana is the maintained fork and what current Debian,
/// Ubuntu and Fedora ship; `libappindicator3` is the older name still present on long-term
/// releases. Trying only the first would report "no tray" on a machine that has one.
pub const APPINDICATOR: [&str; 2] = ["libayatana-appindicator3.so.1", "libappindicator3.so.1"];

/// Ask a loader whether any of the libraries is there.
///
/// The loader is passed in rather than called directly so that this can be checked against
/// a stand-in: the real one needs a Linux desktop, and the rule it implements does not.
pub fn probe_appindicator(loads: impl Fn(&str) -> bool) -> TrayState {
    if APPINDICATOR.iter().any(|name| loads(name)) {
        TrayState::Installed
    } else {
        TrayState::Unavailable
    }
}

/// Whether this machine can show a tray icon.
#[cfg(windows)]
pub fn probe() -> TrayState {
    // The notification area is part of the shell and is always there.
    TrayState::Installed
}

/// Whether this machine can show a tray icon.
#[cfg(target_os = "linux")]
pub fn probe() -> TrayState {
    probe_appindicator(|name| unsafe {
        let Ok(c_name) = std::ffi::CString::new(name) else {
            return false;
        };
        // RTLD_LAZY: nothing here is called, only found. Binding the symbols would cost
        // time and could fail on a library that is present and usable.
        const RTLD_LAZY: i32 = 1;
        let handle = dlopen(c_name.as_ptr(), RTLD_LAZY);
        if handle.is_null() {
            return false;
        }
        dlclose(handle);
        true
    })
}

// Declared by hand rather than pulled in with a crate, the way `tasks::process` declares the
// signal calls it needs: two symbols do not justify a dependency, and since glibc 2.34 they
// live in the C library itself.
//
// A plain comment, not a doc comment: rustdoc generates nothing for an extern block, so `///`
// here is an error under `-D warnings`. Invisible from Windows — the whole block is behind
// `cfg(target_os = "linux")`, so nothing local compiles it at all.
#[cfg(target_os = "linux")]
extern "C" {
    fn dlopen(filename: *const std::os::raw::c_char, flag: i32) -> *mut std::os::raw::c_void;
    fn dlclose(handle: *mut std::os::raw::c_void) -> i32;
}

// ---------- putting the icon there (T395) ----------

/// The labels the tray menu shows, handed in by the interface.
///
/// **The tray is the one place in the core that puts words on a screen**, and it must not
/// write any of its own (`the_tray_puts_no_words_of_its_own_on_the_screen`). A menu entry has
/// to say something, and the nearest string is right there — which is exactly why the rule is
/// checked rather than trusted. These arrive from `src/shared/i18n`, in whatever language the
/// person chose, and change when they change it.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct Labels {
    pub show: String,
    pub quit: String,
}

/// What the menu items are called internally. Not shown to anybody; matched on a click.
pub const SHOW: &str = "show";
pub const QUIT: &str = "quit";

#[cfg(desktop)]
mod desktop {
    use super::{Labels, QUIT, SHOW};
    use tauri::menu::{Menu, MenuItem};
    use tauri::tray::TrayIconBuilder;
    use tauri::{AppHandle, Manager, Runtime};

    /// Build the icon, or replace the one already there.
    ///
    /// **A menu is not optional.** On Linux an AppIndicator with no menu may not be drawn at
    /// all — the icon exists, nothing appears, and the window has been hidden behind it. The
    /// menu is also the only way back on a desktop where clicking the icon does nothing:
    /// `TrayIconEvent` is documented as unsupported there (R-35), so a click cannot be relied
    /// on and "Show" has to be a menu entry.
    ///
    /// The icon is the window's own, from `default_window_icon`. Where there is none there is
    /// nothing to show and this does nothing rather than putting up a blank square.
    pub fn install<R: Runtime>(app: &AppHandle<R>, labels: &Labels) -> tauri::Result<()> {
        let show = MenuItem::with_id(app, SHOW, &labels.show, true, None::<&str>)?;
        let quit = MenuItem::with_id(app, QUIT, &labels.quit, true, None::<&str>)?;
        let menu = Menu::with_items(app, &[&show, &quit])?;

        // Already there: only the words changed, and rebuilding the icon would make it
        // flicker out of the panel and back in on every change of language.
        if let Some(existing) = app.tray_by_id(super::ID) {
            existing.set_menu(Some(menu))?;
            return Ok(());
        }

        let Some(icon) = app.default_window_icon().cloned() else {
            return Ok(());
        };

        TrayIconBuilder::with_id(super::ID)
            .icon(icon)
            .menu(&menu)
            // The menu on a left click as well: on Windows the left button is what people
            // try first, and a menu that only answers the right one reads as a dead icon.
            .show_menu_on_left_click(true)
            .on_menu_event(|app, event| match event.id.as_ref() {
                SHOW => show_the_window(app),
                QUIT => app.exit(0),
                _ => {}
            })
            .build(app)?;
        Ok(())
    }

    /// Bring the window back and put it in front.
    ///
    /// Unhiding is not enough on either platform: a window hidden while another application
    /// was in front comes back behind it, and a person who pressed "Show" and saw nothing
    /// presses it again.
    pub fn show_the_window<R: Runtime>(app: &AppHandle<R>) {
        if let Some(window) = app.get_webview_window("main") {
            let _ = window.show();
            let _ = window.unminimize();
            let _ = window.set_focus();
        }
    }
}

#[cfg(desktop)]
pub use desktop::{install, show_the_window};

/// The identifier the icon is found by when its labels change.
pub const ID: &str = "vrcast-main";
