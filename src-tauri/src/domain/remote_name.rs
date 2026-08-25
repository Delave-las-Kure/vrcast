//! T080 — names during an upload: where a file is assembled and how it enters service
//! (FR-033, FR-038, FR-039).
//!
//! The chief rule: **a half-transferred file must not lie where things are served
//! from.** A web server hands out everything it sees in the serving directory, and
//! counting on a viewer not guessing the name will not do — the application itself
//! hands out the link, and the name is predictable. So a file is assembled in a
//! separate directory and enters service by a single rename.
//!
//! A rename is atomic **only within one file system**. Across a boundary it turns into
//! a copy followed by a delete — which is exactly what we are avoiding: several
//! minutes during which a half-copied file lies in the serving directory. So the
//! staging directory is chosen beside the serving directory, and that they share a
//! file system is checked before the transfer starts.

/// The name of the directory where files are assembled before entering service.
///
/// Placed beside the serving directory — under a shared parent, and so almost
/// certainly on the same file system. It starts with a dot so as not to be an eyesore
/// to anyone who comes to the server by hand.
pub const STAGING_DIR_NAME: &str = ".vrcast-uploads";

/// Where to assemble a file before it enters service.
///
/// Returns a directory beside the serving directory. If the serving directory has no
/// parent (someone gave the root) the answer is `None`: there is nowhere to assemble,
/// and quietly putting it into service is not allowed.
pub fn staging_dir(video_dir: &str) -> Option<String> {
    let trimmed = video_dir.trim_end_matches('/');
    let parent = trimmed.rsplit_once('/')?.0;
    if parent.is_empty() {
        // The serving directory sits at the root: there is nothing to put beside it.
        return None;
    }
    Some(format!("{parent}/{STAGING_DIR_NAME}"))
}

/// The name of the staged file for an upload under this name.
///
/// It depends **only on the final name**, not on a task id. That matters: the whole
/// resume scheme rests on the rule "the position is the size of the staged file", and
/// it has to be findable before a task exists (the pre-flight checks) and after the
/// application restarts.
///
/// The other side of that is two simultaneous uploads under one name writing into one
/// file and quietly ruining each other's work. A name does not solve it: even with a
/// task id in the name, the second upload would still overwrite the first one's result
/// when entering service. So simultaneity is forbidden outright — see the check in
/// `commands::upload`.
pub fn staging_file(staging_dir: &str, remote_name: &str) -> String {
    format!(
        "{}/{}.part",
        staging_dir.trim_end_matches('/'),
        sanitize(remote_name)
    )
}

/// Strip from a name everything that stops it being a name: path separators and line
/// breaks.
///
/// The name comes from a person and goes both into a path on the server and into a
/// command. A slash would take the file into another directory, and a line break would
/// turn one command into two.
pub fn sanitize(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            '/' | '\\' | '\n' | '\r' | '\0' => '_',
            other => other,
        })
        .collect::<String>()
        .trim()
        .trim_start_matches('.')
        .to_owned()
}

/// Whether a name is fit for serving.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NameVerdict {
    Ok,
    /// An empty name, or one made entirely of characters that are not allowed.
    Empty,
    /// The name belongs to an internal serving entry — not to be touched.
    Reserved,
    /// A file of that name is already being served.
    ///
    /// Not a prohibition but grounds for a warning: replacing is legitimate, and it
    /// has consequences (FR-039). `cdn_cached` is true when a CDN is configured: then
    /// viewers will be served the old contents for a while, and a person has to know
    /// that **before** replacing rather than after the complaints.
    Exists {
        cdn_cached: bool,
    },
}

/// Check a name before uploading.
pub fn check_name(name: &str, existing: &[String], cdn_configured: bool) -> NameVerdict {
    let clean = sanitize(name);
    if clean.is_empty() {
        return NameVerdict::Empty;
    }
    if crate::server::SERVICE_ENTRIES.contains(&clean.as_str()) {
        return NameVerdict::Reserved;
    }
    if existing.iter().any(|e| e == &clean) {
        return NameVerdict::Exists {
            cdn_cached: cdn_configured,
        };
    }
    NameVerdict::Ok
}
