//! T678 — a quality set's prepared rung files belong to its medium (the owner's decision of
//! 2026-10-02).
//!
//! A set is cut from one prepared file per rung, `{slug}_{N}.mp4` (or `{slug}_{N}v.mp4` …,
//! T677), lying at the top of the serving directory beside `{slug}/`. Until T678 the
//! catalogue knew only `{slug}/master.m3u8`, so the rung files showed up as «not recognised»
//! and stayed behind when the medium was deleted. Now a build records them under the medium
//! (`Media::set_files`), and deleting the medium removes them with `{slug}/`.
//!
//! **Sets built before T678** have no such record. Their rung files are attributed to the
//! medium whenever the library is read — **only by matching them against the set itself**,
//! never by a name alone: a file is the set's when the set's own record `.prepared` names it,
//! or when it is `{slug}_{N}[v[k]].mp4` for a rung `v{N}` the set's `master.m3u8` serves.
//! **The attribution is written into the catalogue once, at the first read that finds it**
//! (T679, the owner's decision of 2026-10-02) — through the same write every other change
//! takes (`manifest_io::write`: the generation, the sum, the lock), only on a server that
//! already keeps this application's catalogue and that this application may change, and only
//! when the attribution adds something the catalogue does not record ([`to_record`]). A read
//! of somebody else's machine, of a server with no catalogue, or from the cache writes
//! nothing; a write that fails leaves the read as it was and is tried again at the next read.
//!
//! **One rule holds every set file, recorded or found**: it is named as a rung of its own
//! medium's short name (`ladder_build::rung_mbit_of`), it is on the server, and no medium has
//! it as its own file or set. A name that breaks the rule — a medium's single file named like
//! a rung (T675), a record an older copy of the application left behind across a rename — is
//! not the set's, is not shown under it and is never deleted with it.
//!
//! Pure: what is on the server comes in as arguments.

use super::ladder_build::{parse_prepared, rung_mbit_of, PREPARED_RECORD};
use super::manifest::Manifest;

/// What one quality set says about itself, read off the server: `{slug}/master.m3u8` and
/// `{slug}/.prepared`, as text (empty when absent).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SetRecord {
    /// The medium the set is filed under, by its place in the catalogue.
    pub media_index: usize,
    /// The set's directory: the medium's short name.
    pub slug: String,
    pub master: String,
    pub prepared: String,
}

/// The whole megabits of every rung the master serves (`v9/stream.m3u8` → 9).
pub fn rung_mbits_in_master(master: &str) -> Vec<u64> {
    let mut out: Vec<u64> = super::hls_master::playlist_paths(master)
        .iter()
        .filter_map(|p| p.split_once('/').map(|(sub, _)| sub))
        .filter_map(|sub| sub.strip_prefix('v'))
        .filter(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
        .filter_map(|n| n.parse().ok())
        .collect();
    out.sort_unstable();
    out.dedup();
    out
}

/// Which of `candidates` are rung files of the set `slug`, by the set's own word: named by
/// its `.prepared`, or `{slug}_{N}[v[k]].mp4` for an `N` its master serves. Sorted.
///
/// A name alone is never enough: `film_7.mp4` beside a set serving 9 and 4 is not the set's.
pub fn rung_files_of_set(
    slug: &str,
    master: &str,
    prepared: &str,
    candidates: &[&str],
) -> Vec<String> {
    let served = rung_mbits_in_master(master);
    let recorded: Vec<String> = parse_prepared(prepared)
        .into_iter()
        .map(|(_, file)| file)
        .collect();
    let mut out: Vec<String> = candidates
        .iter()
        .filter(|name| match rung_mbit_of(slug, name) {
            Some(mbit) => served.contains(&mbit) || recorded.iter().any(|r| r == *name),
            None => false,
        })
        .map(|name| (*name).to_owned())
        .collect();
    out.sort();
    out.dedup();
    out
}

/// Whether `name` may stand as a set file of a medium of short name `slug` — the rule of the
/// module.
fn fits(slug: &str, name: &str, present: &[&str], owned: &[&str]) -> bool {
    rung_mbit_of(slug, name).is_some() && present.contains(&name) && !owned.contains(&name)
}

/// The catalogue with every medium's `set_files` as they are on the server now.
///
/// - A recorded set file that breaks the rule of the module is left out.
/// - A rung file of a set in `sets` that nobody has is added to that set's medium.
///
/// `present` — the names of the top-level **files** of the serving directory. The generation
/// is not touched: this is a view of the catalogue, not a change to it.
pub fn adopt(manifest: &Manifest, sets: &[SetRecord], present: &[&str]) -> Manifest {
    let mut out = manifest.clone();
    let owned = manifest.all_claimed_paths();
    for m in &mut out.media {
        let slug = m.slug.clone();
        let mut kept: Vec<String> = Vec::new();
        for f in m.set_files.drain(..) {
            if fits(&slug, &f, present, &owned) && !kept.contains(&f) {
                kept.push(f);
            }
        }
        m.set_files = kept;
    }
    for set in sets {
        let Some(slug) = out.media.get(set.media_index).map(|m| m.slug.clone()) else {
            continue;
        };
        if slug != set.slug {
            continue;
        }
        let recorded: Vec<String> = out
            .media
            .iter()
            .flat_map(|m| m.set_files.iter().cloned())
            .collect();
        let candidates: Vec<&str> = present
            .iter()
            .copied()
            .filter(|f| fits(&slug, f, present, &owned) && !recorded.iter().any(|r| r == f))
            .collect();
        let found = rung_files_of_set(&slug, &set.master, &set.prepared, &candidates);
        out.media[set.media_index].set_files.extend(found);
    }
    for m in &mut out.media {
        m.set_files.sort();
    }
    out
}

/// Record the rung files a successful build of the set `slug` made under the medium of that
/// short name (T678), in a catalogue already prepared for writing. Returns whether anything
/// was added.
///
/// The same rule as everywhere: a file a medium has as its own — a single file the build took
/// whole (T675) — stays a single file. Earlier set files are kept: a rung a rebuild left out
/// is still on the server, still this set's, and goes when the medium goes.
pub fn record_built(next: &mut Manifest, slug: &str, built: &[String]) -> bool {
    let Some(index) = next.media.iter().position(|m| m.slug == slug) else {
        return false;
    };
    let owned: Vec<String> = next
        .all_claimed_paths()
        .into_iter()
        .map(str::to_owned)
        .collect();
    let fresh: Vec<String> = built
        .iter()
        .filter(|f| rung_mbit_of(slug, f).is_some() && !owned.contains(f))
        .filter(|f| !next.media[index].set_files.contains(f))
        .cloned()
        .collect();
    if fresh.is_empty() {
        return false;
    }
    for m in &mut next.media {
        m.set_files.retain(|f| !fresh.contains(f));
    }
    let files = &mut next.media[index].set_files;
    files.extend(fresh);
    files.sort();
    files.dedup();
    true
}

/// The catalogue to write so that it records what reading it attributed (T679), or `None`
/// when there is nothing to add.
///
/// `recorded` — the catalogue as read off the server; `seen` — [`adopt`]'s view of it. Only
/// **additions** are written: a rung file the view credits a medium with and the catalogue
/// does not record under it. What the view leaves out of a record (a file gone from the
/// server, a name that no longer fits) is not taken out of the catalogue by a read — the view
/// already hides it, and a file missing for a moment (a rebuild under way) is not forgotten
/// for good. A name written under one medium is taken from any other that records it: one
/// name, one medium. The result is prepared for writing over `recorded.generation`.
///
/// A view that does not match the catalogue medium by medium (another catalogue altogether)
/// writes nothing.
pub fn to_record(recorded: &Manifest, seen: &Manifest) -> Option<Manifest> {
    if recorded.media.len() != seen.media.len()
        || recorded
            .media
            .iter()
            .zip(&seen.media)
            .any(|(r, s)| r.id != s.id || r.slug != s.slug)
    {
        return None;
    }
    let additions: Vec<(usize, Vec<String>)> = recorded
        .media
        .iter()
        .zip(&seen.media)
        .enumerate()
        .filter_map(|(i, (r, s))| {
            let fresh: Vec<String> = s
                .set_files
                .iter()
                .filter(|f| !r.set_files.contains(f))
                .cloned()
                .collect();
            (!fresh.is_empty()).then_some((i, fresh))
        })
        .collect();
    if additions.is_empty() {
        return None;
    }
    let mut next = recorded.prepared_for_write();
    for (index, fresh) in additions {
        for (i, m) in next.media.iter_mut().enumerate() {
            if i != index {
                m.set_files.retain(|f| !fresh.contains(f));
            }
        }
        let files = &mut next.media[index].set_files;
        files.extend(fresh);
        files.sort();
        files.dedup();
    }
    Some(next)
}

/// The sets whose records are to be read: `(media_index, slug)` for every medium that has a
/// ladder `{slug}/master.m3u8` under its own short name and whose directory is on the server.
/// `only` narrows it to one medium.
pub fn sets_to_read(
    manifest: &Manifest,
    present_dirs: &[&str],
    only: Option<usize>,
) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    for (i, m) in manifest.media.iter().enumerate() {
        if only.is_some_and(|o| o != i) {
            continue;
        }
        let has_own_set = m.ladders.iter().any(|l| {
            l.trim_matches('/')
                .split_once('/')
                .is_some_and(|(top, _)| top == m.slug)
        });
        if has_own_set && present_dirs.contains(&m.slug.as_str()) {
            out.push((i, m.slug.clone()));
        }
    }
    out
}

/// Where one part of [`records_script`]'s answer ends and the next begins.
const MARK: &str = "--vrcast-set-record--";

/// One command that prints, for every set, its `master.m3u8` and its `.prepared`, each after
/// a line of its own marking where it begins.
pub fn records_script(video_dir: &str, slugs: &[String], quote: impl Fn(&str) -> String) -> String {
    let dir = video_dir.trim_end_matches('/');
    let mut script = String::new();
    for slug in slugs {
        let master = quote(&format!("{dir}/{slug}/master.m3u8"));
        let prepared = quote(&format!("{dir}/{slug}/{PREPARED_RECORD}"));
        script.push_str(&format!(
            "echo '{MARK}'; cat {master} 2>/dev/null; echo; echo '{MARK}'; cat {prepared} 2>/dev/null; echo; "
        ));
    }
    script.push_str("true");
    script
}

/// Read [`records_script`]'s answer back: `(master, prepared)` per set, in order. A set the
/// answer has nothing for reads as empty — and so attributes nothing.
pub fn parse_records(stdout: &str, count: usize) -> Vec<(String, String)> {
    let separator = format!("{MARK}\n");
    let mut parts = stdout.split(&separator).skip(1);
    (0..count)
        .map(|_| {
            let master = parts.next().unwrap_or_default().to_owned();
            let prepared = parts.next().unwrap_or_default().to_owned();
            (master, prepared)
        })
        .collect()
}
