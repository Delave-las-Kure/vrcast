//! T033 — recovering the grouping from file names (FR-015).
//!
//! The problem: there are files on the server that were uploaded outside the
//! application — by scripts, by hand, by the old way of working. There is no catalogue
//! for them. The application must not hide them, but show them and, where it can,
//! suggest what is what.
//!
//! What it leans on is the naming convention the existing scripts work by: the
//! variants of one work are called `<name>_<bitrate>.mp4` (`Backrooms_10.mp4`,
//! `Backrooms_22.mp4`, `Backrooms_35.mp4`), and a quality ladder sits in a directory
//! `<name>/`.
//!
//! **A suggestion, not a decision.** Nothing is attached automatically: a guessed
//! connection written into the catalogue without asking later diverges from what the
//! person meant, and untangling that is harder than grouping by hand.

/// A suggested group of files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuggestedGroup {
    /// The common part of the name — a ready `slug`, should the person agree.
    pub key: String,
    /// The title to show: the same common part, made readable.
    pub suggested_title: String,
    /// The file paths, relative to the video directory. In the original order.
    pub files: Vec<String>,
    /// Why the files were brought together — shown to the person so they need not
    /// guess where the suggestion came from.
    pub reason: GroupReason,
}

/// The grounds on which files were brought into a group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupReason {
    /// They are in one directory — usually a quality ladder.
    SameDirectory,
    /// The names differ only in the number after the underscore — bitrate variants.
    BitrateVariants,
}

impl GroupReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::SameDirectory => "they are in one directory",
            Self::BitrateVariants => "bitrate variants of one file",
        }
    }
}

/// What the analysis found.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Suggestion {
    /// Files that could be brought together.
    pub groups: Vec<SuggestedGroup>,
    /// Files for which no grounds were found. Shown one by one — but shown, without
    /// fail (FR-015).
    pub singles: Vec<String>,
}

impl Suggestion {
    /// How many files were accounted for in all. It serves as a completeness check:
    /// no file may be lost between the groups and the singletons (FR-015, and the
    /// success criterion about completeness).
    pub fn total_files(&self) -> usize {
        self.groups.iter().map(|g| g.files.len()).sum::<usize>() + self.singles.len()
    }
}

/// Suggest a grouping for files the catalogue knows nothing about.
///
/// The input order is kept in the output: a person sees the list in the same order as
/// on the server, and does not have to hunt for a familiar name again.
pub fn suggest(paths: &[String]) -> Suggestion {
    // Key to (grounds, file indices). Keys are ordered by first appearance.
    let mut order: Vec<String> = Vec::new();
    let mut buckets: std::collections::HashMap<String, (GroupReason, Vec<String>)> =
        std::collections::HashMap::new();
    let mut singles: Vec<String> = Vec::new();

    for path in paths {
        match classify(path) {
            Some((key, reason)) => {
                let entry = buckets.entry(key.clone()).or_insert_with(|| {
                    order.push(key.clone());
                    (reason, Vec::new())
                });
                entry.1.push(path.clone());
            }
            None => singles.push(path.clone()),
        }
    }

    let mut groups = Vec::new();
    for key in order {
        let Some((reason, files)) = buckets.remove(&key) else {
            continue;
        };
        // One file is not a group. A lone `Backrooms_22.mp4` with no neighbours
        // proves nothing, and there are no grounds for creating a medium for it
        // unbidden. A directory is another matter: it is the stated connection.
        if files.len() == 1 && reason == GroupReason::BitrateVariants {
            singles.extend(files);
            continue;
        }
        groups.push(SuggestedGroup {
            suggested_title: readable_title(&key),
            key,
            files,
            reason,
        });
    }

    Suggestion { groups, singles }
}

/// Which group a path belongs to, and why.
fn classify(path: &str) -> Option<(String, GroupReason)> {
    let normalized = path.trim_matches('/');
    if normalized.is_empty() {
        return None;
    }

    // A path with a directory: the directory is the stated connection.
    if let Some((dir, _rest)) = normalized.split_once('/') {
        if !dir.is_empty() {
            return Some((dir.to_owned(), GroupReason::SameDirectory));
        }
    }

    // A name of the form `<common part>_<number>.<extension>`.
    let stem = normalized.rsplit_once('.').map_or(normalized, |(s, _)| s);
    let (prefix, tail) = stem.rsplit_once('_')?;
    if prefix.is_empty() || tail.is_empty() || !tail.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    Some((prefix.to_owned(), GroupReason::BitrateVariants))
}

/// A readable title from the common part of a name: separators become spaces.
fn readable_title(key: &str) -> String {
    let spaced: String = key
        .chars()
        .map(|c| if c == '_' || c == '-' { ' ' } else { c })
        .collect();
    let collapsed = spaced.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        key.to_owned()
    } else {
        collapsed
    }
}
