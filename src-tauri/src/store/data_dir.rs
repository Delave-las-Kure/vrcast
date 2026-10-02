//! Where the application keeps its things on this machine — the one place that says so.
//!
//! **Why one place.** The database, the log beside it and the tables of places each used to
//! ask `directories::ProjectDirs` for themselves. Three askers are three places to forget when
//! the answer has to change, and it has to change for exactly one kind of build: the e2e one.
//!
//! **The e2e build (Cargo feature `e2e`, off by default).** On Windows `ProjectDirs` asks the
//! system for the known folder and ignores the environment entirely, and secrets go to the
//! Windows Credential Manager. An end-to-end run on a working machine would therefore open the
//! person's REAL profiles and write test keys into their REAL credential store — which has
//! already happened on this machine once. In a build with the feature, and only there:
//!
//! - the data directory is [`ENV`] (`VRCAST_DATA_DIR`), an absolute path; a build with the
//!   feature and without the variable has **no** data directory and refuses to start, rather
//!   than falling back to the real one;
//! - secrets go to a file inside that directory (`store::secrets::FileSecretStore`), never to
//!   the operating system's store;
//! - the single-instance lock is not taken (see `lib.rs`): with it, a test instance started
//!   while the person's own application runs would hand its window to theirs and exit. The
//!   webview's own folder (WebView2 keeps it under `%LOCALAPPDATA%\ru.vrcast.studio`) is the
//!   harness's to move — `WEBVIEW2_USER_DATA_FOLDER`, read by WebView2 itself.
//!
//! Without the feature the variable is not read at all: the answer is `ProjectDirs`, byte for
//! byte what it was. `tests/unit/data_dir.rs` holds both halves, and that the release
//! configuration never turns the feature on.

use std::path::PathBuf;

/// The variable an e2e build takes its data directory from. Read only with the `e2e` feature.
pub const ENV: &str = "VRCAST_DATA_DIR";

/// Where an e2e build's ladder check asks for a set (`AppState::verify_origin`): the plain
/// HTTP of the throwaway container the harness started, which has no domain and no
/// certificate. Read only with the `e2e` feature; the application never sets it otherwise.
pub const VERIFY_ORIGIN_ENV: &str = "VRCAST_E2E_VERIFY_ORIGIN";

/// The e2e build's check origin, when the harness gave one (`http://` or `https://` only).
#[cfg(feature = "e2e")]
pub fn e2e_verify_origin() -> Option<String> {
    std::env::var(VERIFY_ORIGIN_ENV)
        .ok()
        .filter(|o| o.starts_with("http://") || o.starts_with("https://"))
}

/// Whether this binary is the e2e build.
pub const E2E: bool = cfg!(feature = "e2e");

/// Text only the e2e binary carries (see `lib.rs::run`): the harness refuses a binary
/// without it. Compiled in only with the feature.
#[cfg(feature = "e2e")]
pub const E2E_MARKER: &str = "VRCAST-E2E-BUILD-a7c3f19e";

/// The application's data directory: the database, `logs/`, the tables of places.
///
/// `None` when it cannot be had — the start-up failure window then says so.
pub fn root() -> Option<PathBuf> {
    #[cfg(feature = "e2e")]
    {
        from_env(std::env::var_os(ENV))
    }
    #[cfg(not(feature = "e2e"))]
    {
        system()
    }
}

/// What the system names — the answer of every ordinary build.
pub fn system() -> Option<PathBuf> {
    directories::ProjectDirs::from("ru", "VRCast", "VRCast Studio")
        .map(|d| d.data_dir().to_path_buf())
}

/// The e2e rule, apart from the environment so it can be checked: only an absolute, non-empty
/// path counts. A relative one would land wherever the driver happened to start the binary.
pub fn from_env(value: Option<std::ffi::OsString>) -> Option<PathBuf> {
    let value = value?;
    if value.is_empty() {
        return None;
    }
    let path = PathBuf::from(value);
    path.is_absolute().then_some(path)
}
