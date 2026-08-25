//! T051 — reconciling the catalogue with what is actually on the server (FR-018).
//!
//! Divergence is inevitable and normal: files are uploaded by scripts, deleted by
//! hand, renamed in a file manager. The application has no right either to pretend
//! that does not happen or to quietly bend the catalogue to fit the facts.
//!
//! Two kinds of divergence and two different answers:
//!
//! - **In the catalogue, not on the server** — the file is marked missing but does not
//!   vanish from its medium. Removing it quietly would hide a loss from the person.
//! - **On the server, not in the catalogue** — the file goes into the "not recognised"
//!   group (FR-015). Hiding it is not allowed: it takes up room and is served over a
//!   link.
//!
//! Only the reconciliation lives here — a pure function of the catalogue and the
//! directory listing. No network, no files: reconciliation is checked without a server,
//! because losing a file is easy precisely here, and such a loss should be caught by a
//! test rather than by a person.

use super::listing::Entry;
use crate::domain::manifest::Manifest;
use std::collections::{HashMap, HashSet};

/// The result of reconciling.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Reconciled {
    /// For each medium, its files marked with whether they exist.
    /// The order of media and files comes from the catalogue.
    pub media_files: Vec<MediaFiles>,
    /// Directory entries that belong to no medium.
    pub unrecognized: Vec<Entry>,
}

/// The files of one medium after reconciling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaFiles {
    pub media_id: String,
    /// The path, the size, and whether it exists on the server.
    pub files: Vec<ResolvedFile>,
    /// Quality ladders: the path and whether it exists.
    pub ladders: Vec<ResolvedFile>,
}

/// A catalogue file matched against the facts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedFile {
    pub path: String,
    pub size_bytes: u64,
    /// False = listed in the catalogue but absent from the server (FR-018).
    pub exists: bool,
}

/// Reconcile the catalogue with the directory listing.
///
/// `entries` is the whole top level of the serving directory, service entries
/// included: the sifting happens here, in one place.
pub fn reconcile(manifest: &Manifest, entries: &[Entry]) -> Reconciled {
    // What is really there, by top-level name.
    let present: HashMap<&str, &Entry> = entries
        .iter()
        .filter(|e| !super::SERVICE_ENTRIES.contains(&e.name.as_str()))
        .map(|e| (e.name.as_str(), e))
        .collect();

    // Which top-level entries the catalogue claims. A catalogue path may be nested
    // (`backrooms/master.m3u8`) — what it claims is the top-level entry `backrooms`.
    let mut claimed: HashSet<&str> = HashSet::new();
    let mut media_files = Vec::new();

    for media in &manifest.media {
        let resolve = |path: &String| -> ResolvedFile {
            let top = top_level(path);
            // For a nested path the size of the top-level entry is the size of the
            // whole quality ladder; it must not be attributed to one description.
            let entry = present.get(top);
            let nested = top != path.as_str();
            ResolvedFile {
                path: path.clone(),
                size_bytes: if nested {
                    0
                } else {
                    entry.map_or(0, |e| e.size_bytes)
                },
                exists: entry.is_some(),
            }
        };

        let files: Vec<ResolvedFile> = media.files.iter().map(&resolve).collect();
        let ladders: Vec<ResolvedFile> = media.ladders.iter().map(&resolve).collect();

        for path in media.all_paths() {
            claimed.insert(top_level(path));
        }

        media_files.push(MediaFiles {
            media_id: media.id.clone(),
            files,
            ladders,
        });
    }

    // The order of the unrecognised comes from the directory listing rather than from
    // a set: a person sees this list, and it must not change from run to run.
    let unrecognized: Vec<Entry> = entries
        .iter()
        .filter(|e| !super::SERVICE_ENTRIES.contains(&e.name.as_str()))
        .filter(|e| !claimed.contains(e.name.as_str()))
        .cloned()
        .collect();

    Reconciled {
        media_files,
        unrecognized,
    }
}

/// The top-level entry a path belongs to.
fn top_level(path: &str) -> &str {
    let trimmed = path.trim_matches('/');
    trimmed.split_once('/').map_or(trimmed, |(head, _)| head)
}
