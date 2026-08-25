//! T089 — comparing checksums before a file goes into service (FR-032).
//!
//! Why bother when the transfer runs over a reliable connection: the channel is
//! reliable, not the whole chain. Between the local disk and the server's there is
//! also reading the file, writing at an offset across several attempts, resuming after
//! breaks, and the server's file system. Each link usually tells the truth — but
//! "usually" is not enough here: a corrupted file in service breaks viewing quietly,
//! and by then there will be nothing left to connect it to an upload a month ago.
//!
//! The sum is computed **on the server, by the server's own tools**, not by the code
//! that transferred the file. Checking a result with the same instrument that produced
//! it means checking that the code repeats its own mistake reliably.

use crate::server::shell_quote;
use crate::ssh::{Connection, Result, SshError};
use sha2::{Digest, Sha256};
use std::path::Path;

/// How much is read at a time when summing a local file.
const READ_CHUNK: usize = 1024 * 1024;

/// Compute the sum of a local file.
///
/// The file may be tens of gigabytes, so it is read in pieces rather than wholly into
/// memory. The work blocks, and so it is moved off the task runtime: otherwise it
/// would hold for minutes the thread every other task runs on.
pub async fn local(path: &Path) -> std::io::Result<String> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || {
        use std::io::Read;

        let mut file = std::fs::File::open(&path)?;
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; READ_CHUNK];
        loop {
            let read = file.read(&mut buf)?;
            if read == 0 {
                break;
            }
            hasher.update(&buf[..read]);
        }
        Ok(hex::encode(hasher.finalize()))
    })
    .await
    .map_err(|e| std::io::Error::other(e.to_string()))?
}

/// Compute the sum of a file on the server, using the server's own tools.
pub async fn remote(conn: &Connection, path: &str) -> Result<String> {
    let out = conn
        .exec(&format!("sha256sum -- {}", shell_quote(path)))
        .await?;
    if !out.ok() {
        return Err(SshError::Exec(format!(
            "could not compute the checksum on the server: {}",
            out.stderr.trim()
        )));
    }

    // The output is "<sum>  <name>". The first word is what we want.
    out.trimmed()
        .split_whitespace()
        .next()
        .map(str::to_owned)
        .filter(|s| s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit()))
        .ok_or_else(|| {
            SshError::Exec(format!(
                "sha256sum output could not be parsed: {}",
                out.trimmed().chars().take(80).collect::<String>()
            ))
        })
}

/// Whether the sums match. Compared without regard to case: different tools write
/// hexadecimal differently, and a difference of case is not a difference of value.
pub fn matches(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}
