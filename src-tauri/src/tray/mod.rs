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
