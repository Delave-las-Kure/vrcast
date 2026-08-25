//! T048a, T093 — whether anything is being served right now (FR-019a, FR-037).
//!
//! Needed before two different actions, for one reason: deleting a file and uploading
//! a new one both spoil what is being watched at that moment. Deleting cuts the
//! viewing short; uploading pushes out of the server's memory what it was holding for
//! viewers, and their playback starts to stall.
//!
//! **An honest caveat for milestones A and B.** The specification asks for the number
//! of viewers of a particular file. The server's connection table does not say what is
//! being downloaded, and there is as yet nothing to attribute a connection to a medium
//! with: that needs the serving log parsed, and that is Phase 4. So what comes back is
//! the number of open serving connections — the fact that there are some, which is
//! what tasks T048a and T093 allow. Calling it "viewers of the file" would be telling
//! the user something we do not know.

use crate::ssh::Connection;

/// The ports serving answers viewers on.
const SERVING_PORTS: [u16; 2] = [80, 443];

/// How many connections the web server is serving right now.
///
/// A failed count is zero rather than an error: the warning is useful, but refusing a
/// deletion or an upload over it would be out of proportion. The failure goes to the
/// log.
pub async fn serving_connections(conn: &Connection) -> usize {
    let ports = SERVING_PORTS
        .iter()
        .map(|p| format!("sport = :{p}"))
        .collect::<Vec<_>>()
        .join(" or ");

    let cmd = format!("ss -tn state established '( {ports} )' 2>/dev/null | tail -n +2 | wc -l");

    match conn.exec(&cmd).await {
        Ok(out) if out.ok() => out.trimmed().trim().parse::<usize>().unwrap_or(0),
        Ok(out) => {
            tracing::debug!(stderr = %out.stderr.trim(), "could not count serving connections");
            0
        }
        Err(e) => {
            tracing::debug!(error = %e, "could not count serving connections");
            0
        }
    }
}
