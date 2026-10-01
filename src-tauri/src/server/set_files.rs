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

/// What [`record_found`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recorded {
    /// Nothing to write, or not allowed to write here.
    Nothing,
    /// The catalogue now records what the read attributed.
    Written,
    /// The write was refused or failed; the read goes on as it was, and the next read tries
    /// again.
    Failed,
}

/// Write into the catalogue the set files a read attributed and it does not record (T679).
///
/// `present` — the server keeps this application's catalogue (`library.json` is there);
/// `may_change` — this read is one that may write (not from the cache) on a server this
/// application may change (not somebody else's, not one it is too old for). `recorded` is
/// the catalogue as read, `seen` the read's view of it (`adopted`).
///
/// `write` is handed the catalogue to write and the generation it was read at — in the
/// application, `manifest_io::write`, the one write every change of the catalogue goes
/// through (the generation checked, the bytes summed, the replacement under the catalogue's
/// lock): a catalogue somebody changed meanwhile is refused, never overwritten. **Never an
/// error**: a write that fails is said in the log and the read is answered all the same; it
/// is not repeated here — the next ordinary read tries again.
pub async fn record_found<F, Fut, E>(
    present: bool,
    may_change: bool,
    recorded: &Manifest,
    seen: &Manifest,
    write: F,
) -> Recorded
where
    F: FnOnce(Manifest, u64) -> Fut,
    Fut: std::future::Future<Output = Result<(), E>>,
    E: std::fmt::Display,
{
    if !present || !may_change {
        return Recorded::Nothing;
    }
    let Some(next) = set_files::to_record(recorded, seen) else {
        return Recorded::Nothing;
    };
    match write(next, recorded.generation).await {
        Ok(()) => {
            tracing::info!("the older sets' rung files were recorded in the catalogue");
            Recorded::Written
        }
        Err(e) => {
            tracing::warn!(error = %e, "the older sets' rung files were not recorded; the next read tries again");
            Recorded::Failed
        }
    }
}
