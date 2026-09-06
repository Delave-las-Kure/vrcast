//! T031 — the library catalogue `library.json` with its generation counter
//! (`contracts/server-contract.md`, the library catalogue section).
//!
//! The generation counter exists for one case: two copies of the application working
//! with one server. The order of writing is not optional (R-10): read with the
//! generation, change, write beside it, replace atomically. If the generation on the
//! server has changed in the meantime the write **does not happen** — otherwise the
//! second copy quietly wipes out the first one's work.
//!
//! Only parsing, assembling and the rules live here. Writing to the server itself is
//! in `server::manifest_io`.

use super::media::{validate_slug, Media};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// The library catalogue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    /// Goes up by one on every write. Zero means there was no catalogue yet.
    pub generation: u64,
    #[serde(default)]
    pub media: Vec<Media>,
    /// Fields this application does not know about.
    ///
    /// Deliberately kept when rewriting: the catalogue may have been written by a
    /// newer version of the application, and its data must not be lost (FR-131).
    /// Quietly discarding what is not understood is the quietest way of ruining
    /// somebody else's records.
    #[serde(flatten, default, skip_serializing_if = "HashMap::is_empty")]
    pub extra: HashMap<String, serde_json::Value>,
}

impl Default for Manifest {
    fn default() -> Self {
        Self::empty()
    }
}

/// Why the catalogue could not be read.
#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("catalogue could not be parsed: {0}")]
    Malformed(String),
}

/// What is wrong inside the catalogue. It lives on the server and a person may have
/// edited it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManifestProblem {
    DuplicateId(String),
    DuplicateSlug(String),
    /// One file belongs to two media — deleting one would take it from the other.
    FileClaimedTwice {
        path: String,
        media: Vec<String>,
    },
    EmptyId,
    BadSlug {
        slug: String,
        reason: String,
    },
}

impl std::fmt::Display for ManifestProblem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DuplicateId(id) => write!(f, "two media with the same id {id:?}"),
            Self::DuplicateSlug(slug) => {
                write!(f, "two media with the same short name {slug:?}")
            }
            Self::FileClaimedTwice { path, media } => write!(
                f,
                "the file {path:?} belongs to several media at once: {}",
                media.join(", ")
            ),
            Self::EmptyId => f.write_str("a medium has an empty id"),
            Self::BadSlug { slug, reason } => {
                write!(f, "the short name {slug:?} is not allowed: {reason}")
            }
        }
    }
}

impl Manifest {
    /// An empty catalogue — what a server without a library starts from.
    pub fn empty() -> Self {
        Self {
            generation: 0,
            media: Vec::new(),
            extra: HashMap::new(),
        }
    }

    /// Parse the contents of `library.json`.
    ///
    /// Empty contents mean an absent catalogue rather than an error: on a fresh server
    /// the file does not exist yet, and failing here would declare an empty library a
    /// fault.
    pub fn parse(text: &str) -> Result<Self, ManifestError> {
        if text.trim().is_empty() {
            return Ok(Self::empty());
        }
        serde_json::from_str(text).map_err(|e| ManifestError::Malformed(e.to_string()))
    }

    /// Assemble the contents to write to the server.
    ///
    /// Indented, because people read and edit this file — including when the
    /// application is unavailable and something has to be worked out.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|_| String::from("{}"))
    }

    /// A catalogue for writing over the one that was read: the generation is one
    /// higher.
    ///
    /// A step of its own rather than a `+= 1` wherever it happens to be convenient:
    /// raising the generation *is* the claim "I am writing over what I read", and it
    /// should be made in one place.
    pub fn prepared_for_write(&self) -> Self {
        let mut next = self.clone();
        next.generation = self.generation.saturating_add(1);
        next
    }

    /// Put a path under one medium, taking it out of wherever else it was.
    ///
    /// **One path, one medium, and this is what keeps it so.** `FileClaimedTwice` is a
    /// declared fault of a catalogue — one file belonging to two media, where deleting one
    /// takes it from the other — and the only way to add a path without risking it is to
    /// remove it first. Written once here because it was written twice before: `file_move`
    /// and the end of an upload both did it, in copies that could drift.
    ///
    /// **Safe to repeat** (principle V): filing the same path under the same medium twice
    /// leaves one entry, because the removal runs whether or not the addition finds a home.
    ///
    /// An unknown medium changes nothing and says so. It is not invented: somebody deleted it
    /// between choosing it and the work finishing, and creating it again resurrects a thing
    /// they got rid of.
    #[must_use]
    pub fn with_file_under(&self, media_id: &str, path: &str, is_ladder: bool) -> Option<Self> {
        self.find_by_id(media_id)?;
        let mut next = self.prepared_for_write();
        for m in &mut next.media {
            m.files.retain(|p| p != path);
            m.ladders.retain(|p| p != path);
        }
        let target = next
            .media
            .iter_mut()
            .find(|m| m.id == media_id)
            .expect("the medium was found a moment ago");
        if is_ladder {
            target.ladders.push(path.to_owned());
        } else {
            target.files.push(path.to_owned());
        }
        Some(next)
    }

    /// Whether writing is allowed: is the generation on the server still the one that
    /// was read.
    ///
    /// `base` is the generation read before the change; `current` is what is on the
    /// server now.
    pub fn write_allowed(base: u64, current: u64) -> bool {
        base == current
    }

    pub fn find_by_slug(&self, slug: &str) -> Option<&Media> {
        self.media.iter().find(|m| m.slug == slug)
    }

    pub fn find_by_id(&self, id: &str) -> Option<&Media> {
        self.media.iter().find(|m| m.id == id)
    }

    /// Every file and quality-ladder description belonging to media.
    pub fn all_claimed_paths(&self) -> Vec<&str> {
        self.media
            .iter()
            .flat_map(|m| m.files.iter().chain(m.ladders.iter()))
            .map(String::as_str)
            .collect()
    }

    /// Whether a short name is free (a `slug` is unique within a server).
    ///
    /// `except_id` makes the check usable when renaming: a medium does not clash with
    /// itself.
    pub fn slug_available(&self, slug: &str, except_id: Option<&str>) -> bool {
        !self
            .media
            .iter()
            .any(|m| m.slug == slug && Some(m.id.as_str()) != except_id)
    }

    /// Check the whole catalogue. Returns **every** objection.
    pub fn validate(&self) -> Result<(), Vec<ManifestProblem>> {
        let mut problems = Vec::new();
        let mut seen_ids: HashMap<&str, usize> = HashMap::new();
        let mut seen_slugs: HashMap<&str, usize> = HashMap::new();
        // The owners keep their order: an error message must name them in the same
        // order as the catalogue, or it changes from run to run.
        let mut owners: Vec<(&str, Vec<&str>)> = Vec::new();
        let mut owner_index: HashMap<&str, usize> = HashMap::new();

        for m in &self.media {
            if m.id.trim().is_empty() {
                problems.push(ManifestProblem::EmptyId);
            } else {
                *seen_ids.entry(m.id.as_str()).or_insert(0) += 1;
            }

            match validate_slug(&m.slug) {
                Ok(()) => {
                    *seen_slugs.entry(m.slug.as_str()).or_insert(0) += 1;
                }
                Err(e) => problems.push(ManifestProblem::BadSlug {
                    slug: m.slug.clone(),
                    reason: e.to_string(),
                }),
            }

            for path in m.files.iter().chain(m.ladders.iter()) {
                let idx = *owner_index.entry(path.as_str()).or_insert_with(|| {
                    owners.push((path.as_str(), Vec::new()));
                    owners.len() - 1
                });
                owners[idx].1.push(m.id.as_str());
            }
        }

        for id in sorted_duplicates(&seen_ids) {
            problems.push(ManifestProblem::DuplicateId(id.to_owned()));
        }
        for slug in sorted_duplicates(&seen_slugs) {
            problems.push(ManifestProblem::DuplicateSlug(slug.to_owned()));
        }
        for (path, claimants) in owners {
            if claimants.len() > 1 {
                problems.push(ManifestProblem::FileClaimedTwice {
                    path: path.to_owned(),
                    media: claimants.into_iter().map(str::to_owned).collect(),
                });
            }
        }

        if problems.is_empty() {
            Ok(())
        } else {
            Err(problems)
        }
    }
}

/// Repeated keys in a stable order: error messages must not change from run to run
/// merely because walking a map is arbitrary.
fn sorted_duplicates<'a>(counts: &HashMap<&'a str, usize>) -> Vec<&'a str> {
    let mut dups: Vec<&str> = counts
        .iter()
        .filter(|(_, n)| **n > 1)
        .map(|(k, _)| *k)
        .collect();
    dups.sort_unstable();
    dups
}
