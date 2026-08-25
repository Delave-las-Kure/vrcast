//! T050 — how much room is left on the server's disk (FR-017).
//!
//! Not shown out of curiosity: an upload onto a full disk breaks off halfway, and it
//! is better to learn that before an hour of transfer rather than after (FR-036).

use crate::commands::library::DiskUsage;
use crate::ssh::{Connection, Result, SshError};

/// Read the state of the disk the serving directory sits on.
pub async fn usage(conn: &Connection, video_dir: &str) -> Result<DiskUsage> {
    let dir = super::shell_quote(video_dir);

    // One command rather than two: every call to the server is a network round trip,
    // and the room has to appear with the library, not a second after it.
    //
    // `df -P` gives predictable output: one line per file system, with no wrapping of
    // long device names. Sizes in kilobytes, as POSIX requires — sturdier than asking
    // for bytes with a flag that may not exist.
    let out = conn
        .exec(&format!(
            "df -Pk -- {dir} | tail -n 1; du -sk -- {dir} 2>/dev/null | cut -f1"
        ))
        .await?;

    if !out.ok() {
        return Err(SshError::Exec(format!(
            "could not find out the disk space: {}",
            out.stderr.trim()
        )));
    }

    let mut lines = out.stdout.lines();
    let df_line = lines.next().unwrap_or_default();
    let du_line = lines.next().unwrap_or_default();

    // df output: device, total, used, available, percentage, mount point.
    let fields: Vec<&str> = df_line.split_whitespace().collect();
    let total_kb = fields.get(1).and_then(|v| v.parse::<u64>().ok());
    let free_kb = fields.get(3).and_then(|v| v.parse::<u64>().ok());

    let (Some(total_kb), Some(free_kb)) = (total_kb, free_kb) else {
        return Err(SshError::Exec(format!(
            "df output could not be parsed: {}",
            df_line.trim()
        )));
    };

    // The size of the serving directory is optional: on a very large library that sum
    // takes noticeably longer than the rest, and it is better to show the disk space
    // without it than to show nothing.
    let used_kb = du_line.trim().parse::<u64>().unwrap_or(0);

    Ok(DiskUsage {
        total_bytes: total_kb.saturating_mul(1024),
        free_bytes: free_kb.saturating_mul(1024),
        used_by_videos_bytes: used_kb.saturating_mul(1024),
    })
}
