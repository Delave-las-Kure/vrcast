//! T678 — reading what each quality set says about itself, so that its prepared rung files
//! are attributed to its medium. The rules are in `domain::set_files`; this only asks the
//! server for `{slug}/master.m3u8` and `{slug}/.prepared`, in one command for every set.

use super::listing::Entry;
use crate::domain::manifest::Manifest;
use crate::domain::set_files::{self, SetRecord};
use crate::ssh::Connection;

/// The catalogue with every medium's `set_files` as they are on the server (see
/// `domain::set_files::adopt`). `only` reads the set of one medium alone.
///
/// Best effort: when the sets cannot be read, nothing new is attributed — what the catalogue
/// already records still stands, checked against the listing. The generation is untouched.
pub async fn adopted(
    conn: &Connection,
    video_dir: &str,
    manifest: &Manifest,
    entries: &[Entry],
    only: Option<usize>,
) -> Manifest {
    let dirs: Vec<&str> = entries
        .iter()
        .filter(|e| e.is_dir)
        .map(|e| e.name.as_str())
        .collect();
    let files: Vec<&str> = entries
        .iter()
        .filter(|e| !e.is_dir)
        .map(|e| e.name.as_str())
        .collect();

    let wanted = set_files::sets_to_read(manifest, &dirs, only);
    let mut sets = Vec::new();
    if !wanted.is_empty() {
        let slugs: Vec<String> = wanted.iter().map(|(_, s)| s.clone()).collect();
        let script = set_files::records_script(video_dir, &slugs, super::shell_quote);
        match conn.exec(&script).await {
            Ok(out) => {
                let records = set_files::parse_records(&out.stdout, wanted.len());
                for ((media_index, slug), (master, prepared)) in wanted.into_iter().zip(records) {
                    sets.push(SetRecord {
                        media_index,
                        slug,
                        master,
                        prepared,
                    });
                }
            }
            Err(e) => {
                tracing::debug!(error = %e, "the sets' own records were not read");
            }
        }
    }
    set_files::adopt(manifest, &sets, &files)
}
