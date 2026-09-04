//! T479 — a module nothing can reach is indistinguishable from a module nobody wrote.
//!
//! **The third time in one project.** T366 found three IPC commands registered and called
//! from nowhere; T443 found a whole screen with no way in. Both were caught by the command
//! guard, which compares what the core registers against what the interface calls. This one
//! sits a layer below where that guard looks: `domain::scene_cut` and `media::split` are
//! written, checked, and referenced by nothing but their own tests — no command, no task, no
//! screen. The milestone's core exists and the application cannot use it.
//!
//! So the rule is reachability rather than registration: starting from the two doors the
//! outside world knocks on — `commands` and `tasks` — follow module references until nothing
//! new is found. Whatever is left over is unreachable.
//!
//! **What this deliberately does not do.** It does not judge whether reaching a module is
//! useful, only whether it is possible. A module reached by one dead branch passes; that is
//! the command guard's business, not this one's.

use std::collections::{BTreeSet, VecDeque};
use std::path::{Path, PathBuf};

/// Modules that nothing reaches yet, each with the reason and what closes it.
///
/// **This list is the point of the check, and it must shrink.** An entry says "written, and
/// deliberately not connected yet, because —". An entry with no reason is a module somebody
/// forgot, which is exactly what this is here to find.
const NOT_REACHED_YET: &[(&str, &str)] = &[
    (
        "domain::scene_cut",
        "milestone F: where to cut. Connecting it means a screen, and a screen means deciding \
         what the person is shown after the cut — which depends on what the owner's 3D \
         converter gives back (T454, not measured). Closed by T454 and the screen after it.",
    ),
    (
        "domain::grouping",
        "⚠ not a deferral but a debt, and the entry says so. It is the whole of T033, which is          ticked, and T033 is what FR-015 is claimed by — so a requirement stands as delivered          on a module the application cannot call, and neither half of FR-015 exists anywhere          else: nothing in the commands or the interface knows the words for a file belonging          to no media. Closed by T480, which either connects it or deletes it.",
    ),
    (
        "media::split",
        "milestone F: the cutting and the joining themselves. Same reason as scene_cut, and \
         they go in together: cutting with nothing to join is worse than neither.",
    ),
];

fn rust_files(dir: &Path, into: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, into);
        } else if path.extension().is_some_and(|e| e == "rs") {
            into.push(path);
        }
    }
}

/// Every module of the core, named the way a reference to it is written: `domain::chunks`.
///
/// Only the two subject-matter trees. `commands`, `tasks`, `store`, `ssh` and `server` are
/// the plumbing and the doors; they are where the walk starts, not what it judges.
fn modules() -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    for top in ["domain", "media"] {
        let mut files = Vec::new();
        rust_files(Path::new("src").join(top).as_path(), &mut files);
        for file in files {
            let stem = file.file_stem().unwrap().to_string_lossy().to_string();
            if stem == "mod" {
                continue;
            }
            found.insert(format!("{top}::{stem}"));
        }
    }
    found
}

/// What one file's text mentions, out of the modules given.
fn mentioned(text: &str, all: &BTreeSet<String>) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for m in all {
        let short = m.split("::").nth(1).unwrap();
        // `chunks::shape_of`, `use crate::domain::chunks`, `super::chunks::CHUNK_S`.
        if text.contains(&format!("{short}::")) || text.contains(&format!("::{short};")) {
            out.insert(m.clone());
        }
    }
    out
}

fn text_of(paths: &[PathBuf]) -> String {
    paths
        .iter()
        .filter_map(|p| std::fs::read_to_string(p).ok())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn every_module_of_the_core_can_be_reached_from_a_command_or_a_task() {
    let all = modules();
    assert!(
        all.len() > 20,
        "the walk found almost no modules — it is looking in the wrong place: {all:?}"
    );

    // **The doors are the whole application except the subject matter itself.** An earlier
    // version started only at `commands` and `tasks` and walked forward through `domain` and
    // `media` — and called three modules unreachable that `store::redact` and `server::upload`
    // use every day. A guard that cries wolf teaches people to widen the excuse list, which
    // is worse than not having it.
    let judged: Vec<PathBuf> = all
        .iter()
        .map(|m| {
            let (top, stem) = m.split_once("::").unwrap();
            PathBuf::from(format!("src/{top}/{stem}.rs"))
        })
        .collect();
    let mut doors = Vec::new();
    rust_files(Path::new("src"), &mut doors);
    doors.retain(|f| !judged.iter().any(|j| same_file(j, f)));
    let mut reached: BTreeSet<String> = mentioned(&text_of(&doors), &all);
    let mut queue: VecDeque<String> = reached.iter().cloned().collect();

    // And onwards: a module reached by a reached module is reached.
    while let Some(m) = queue.pop_front() {
        let (top, stem) = m.split_once("::").unwrap();
        let path = PathBuf::from(format!("src/{top}/{stem}.rs"));
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        for next in mentioned(&text, &all) {
            if reached.insert(next.clone()) {
                queue.push_back(next);
            }
        }
    }

    let unreachable: Vec<&String> = all.difference(&reached).collect();
    let excused: BTreeSet<&str> = NOT_REACHED_YET.iter().map(|(m, _)| *m).collect();

    let unexcused: Vec<&&String> = unreachable
        .iter()
        .filter(|m| !excused.contains(m.as_str()))
        .collect();
    assert!(
        unexcused.is_empty(),
        "these modules are written and nothing can reach them — no command, no task, no \
         screen. Either connect them or put them in NOT_REACHED_YET with the reason and what \
         closes it:\n  {}",
        unexcused
            .iter()
            .map(|m| m.as_str())
            .collect::<Vec<_>>()
            .join("\n  ")
    );

    // And the other way round, so the list cannot quietly outlive its reason.
    let stale: Vec<&str> = excused
        .iter()
        .filter(|m| !unreachable.iter().any(|u| u.as_str() == **m))
        .copied()
        .collect();
    assert!(
        stale.is_empty(),
        "NOT_REACHED_YET still excuses modules that are now reachable — take them out, or the \
         list stops meaning anything:\n  {}",
        stale.join("\n  ")
    );
}

/// Whether two paths name the same file, whatever separators they were written with.
fn same_file(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| {
        p.components()
            .map(|c| c.as_os_str().to_string_lossy().to_lowercase())
            .collect::<Vec<_>>()
            .join("/")
    };
    norm(a) == norm(b)
}
