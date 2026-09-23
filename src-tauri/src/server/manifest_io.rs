//! T045 — reading and writing the library catalogue on the server.
//!
//! The order of writing is not optional (R-10, `contracts/server-contract.md`): read
//! with the generation, change, write to a staged file beside it, replace atomically.
//! And before the replacement the generation on the server is checked once more:
//! otherwise a second copy of the application quietly wipes out the first one's work.
//!
//! Why "beside" rather than "over": writing over is a window in which a half-written
//! file sits on the server. Should the connection break exactly there, the library is
//! lost not by half but entirely — there is nothing to parse truncated JSON with.
//!
//! **The check and the replacement are one step on the server** (T604). They used to be
//! three: read the generation, stage the file, then a separate `mv`. Two copies that read
//! the same generation both passed the check before either replaced anything, and the
//! second replacement quietly wiped out the first. Now the replacement is a single `exec`
//! that takes a lock, confirms the catalogue is byte for byte the one this write checked
//! the generation of, and only then moves the staged file in — see `write`.

use sha2::{Digest, Sha256};

use super::{join_remote, shell_quote};
use crate::domain::manifest::Manifest;
use crate::ssh::Connection;

/// The name of the catalogue file inside the serving directory.
pub const MANIFEST_NAME: &str = "library.json";

/// How long a write waits for another write to the same catalogue to finish (T604).
///
/// What another write holds the lock for is one `sha256sum` of the catalogue and one
/// `mv` within a directory — milliseconds, even for a catalogue of thousands of media
/// (measured in the test image, 2026-09-23: lock + sum + compare + `mv` on a 1.3 MB
/// catalogue took 4.7 ms; the whole two-writer round in the concurrent test, SSH included,
/// well under a second). Waiting this long therefore means somebody is
/// holding the lock for something that is not a catalogue write — and the answer to
/// that is "somebody else is changing the library, read again and retry", not a wait
/// with no end. Thirty seconds leaves a slow disk and a slow link a wide margin.
const LOCK_WAIT_SECS: u64 = 30;

/// The exit status `flock` is told to give when `LOCK_WAIT_SECS` runs out, so a timeout
/// is told apart from `flock` failing for any other reason.
const LOCK_TIMEOUT_EXIT: u8 = 75;

/// What stands for "there was no catalogue" where a checksum would be: it can never be
/// mistaken for a sum, which is 64 hexadecimal digits.
const ABSENT: &str = "ABSENT";

#[derive(Debug, thiserror::Error)]
pub enum ManifestIoError {
    /// The catalogue was changed by another copy of the application between the read
    /// and the write. The write **did not happen**: the other change stands.
    #[error("the catalogue was changed by another application: read generation {base}, server has {current}")]
    Conflict { base: u64, current: u64 },

    /// Another write to this catalogue held the lock for longer than `LOCK_WAIT_SECS`
    /// (T604). Nothing was written. For the caller it means the same as `Conflict` —
    /// somebody else is changing the library; read again and retry — and it is reported
    /// under the same code.
    #[error("another change of the catalogue is in progress on this server")]
    Busy,

    #[error("catalogue could not be parsed: {0}")]
    Malformed(String),

    #[error(transparent)]
    Ssh(#[from] crate::ssh::SshError),
}

pub type Result<T> = std::result::Result<T, ManifestIoError>;

/// Read the catalogue. A missing file is an empty library, not an error.
pub async fn read(conn: &Connection, video_dir: &str) -> Result<Manifest> {
    let raw = read_raw(conn, video_dir).await?;
    parse(raw.as_deref())
}

/// The catalogue's bytes exactly as they lie on the server, or `None` if there is no
/// catalogue at all.
async fn read_raw(conn: &Connection, video_dir: &str) -> Result<Option<Vec<u8>>> {
    let path = join_remote(video_dir, MANIFEST_NAME);
    let sftp = conn.sftp().await?;

    match sftp.read(path.clone()).await {
        Ok(b) => Ok(Some(b)),
        // Telling "no file" from "no access" by the shape of the library's error is
        // not reliable enough, so the server is asked directly. Treating any failed
        // read as an empty library is dangerous: the application would decide there
        // is no catalogue and wipe out the real one with its very next write.
        Err(e) => {
            let exists = conn
                .exec(&format!("test -e {}", shell_quote(&path)))
                .await?
                .ok();
            if exists {
                return Err(ManifestIoError::Ssh(crate::ssh::SshError::sftp(
                    crate::store::redact::safe_display(&e),
                )));
            }
            Ok(None)
        }
    }
}

fn parse(raw: Option<&[u8]>) -> Result<Manifest> {
    match raw {
        None => Ok(Manifest::empty()),
        Some(bytes) => {
            let text = String::from_utf8_lossy(bytes);
            Manifest::parse(&text).map_err(|e| ManifestIoError::Malformed(e.to_string()))
        }
    }
}

/// What the catalogue is expected to be at the moment of replacement: the sum of the
/// bytes the generation was checked on, or `ABSENT`.
fn fingerprint_of(raw: Option<&[u8]>) -> String {
    match raw {
        Some(bytes) => hex::encode(Sha256::digest(bytes)),
        None => String::from(ABSENT),
    }
}

/// Write the catalogue if the server still holds `base_generation`.
///
/// `manifest.generation` must be exactly one more than `base_generation` — that is the
/// claim "I am writing over what I read" (see `Manifest::prepared_for_write`).
///
/// **How "still holds" is made true at the moment of replacement** (T604). The
/// generation is checked here, in Rust, on bytes read from the server — and the sum of
/// exactly those bytes goes with the replacement. On the server, in one `exec` and under
/// one lock that every writer of this catalogue takes, the catalogue is summed again, and
/// the staged file is moved in only if the sums agree. A write that lost the race sees a
/// different sum and is refused as a `Conflict`; the winner's catalogue stands.
///
/// Why a sum and not the generation read by the shell: the sum needs no JSON parsing in
/// `sh`, does not care whether the file is pretty-printed or compact, and cannot be
/// fooled by a medium titled `"generation": 7`. Equal sums mean the very bytes the
/// generation was checked on — which is exactly the claim being made.
///
/// **Why the lock is the serving directory itself** rather than a file. A lock file in
/// the serving directory would show in the person's library as "not recognised"
/// (`server::listing` hides only `.` and `..`); one beside it needs write access to a
/// directory this application does not own, and there is none beside a directory at the
/// root; one in `/tmp` can be swept away while held, after which two writers lock two
/// different files and exclude nobody. The directory is there for as long as there is a
/// catalogue in it, its inode does not change, and `flock` locks a directory as well as a
/// file. **Not** the catalogue file: `mv` gives the name a new inode, and a lock on the
/// old one does not stop a writer who opened the new one.
pub async fn write(
    conn: &Connection,
    video_dir: &str,
    manifest: &Manifest,
    base_generation: u64,
) -> Result<()> {
    // The check comes BEFORE the staged file is created. Otherwise a refusal would
    // leave litter in the serving directory — and a person sees that directory as
    // their library.
    let raw = read_raw(conn, video_dir).await?;
    let current = parse(raw.as_deref())?.generation;
    if !Manifest::write_allowed(base_generation, current) {
        return Err(ManifestIoError::Conflict {
            base: base_generation,
            current,
        });
    }
    let expected = fingerprint_of(raw.as_deref());

    let target = join_remote(video_dir, MANIFEST_NAME);
    // The staged file's name belongs to this attempt rather than being shared: two
    // copies that reach the write at the same moment must not write into one file.
    let temp = join_remote(
        video_dir,
        &format!(".{MANIFEST_NAME}.{}.tmp", uuid::Uuid::new_v4().simple()),
    );

    let body = manifest.to_json();
    let sftp = conn.sftp().await?;

    // `create` specifically: the library's `write` opens a file for writing without
    // creating it, and on a path that does not exist gives "no such file". The name
    // promises one thing and the behaviour is another — caught on a live server on
    // 2026-08-25.
    let written = async {
        use tokio::io::AsyncWriteExt;
        let mut file = sftp.create(temp.clone()).await?;
        file.write_all(body.as_bytes()).await?;
        file.flush().await?;
        file.shutdown().await?;
        Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
    }
    .await;

    if let Err(e) = written {
        // Cleaned up by us: a staged file in the serving directory lands in the
        // person's "not recognised" group and alarms them.
        let _ = sftp.remove_file(temp.clone()).await;
        return Err(ManifestIoError::Ssh(crate::ssh::SshError::sftp(
            crate::store::redact::safe_display(&*e),
        )));
    }

    // Replacement by renaming specifically: it is atomic within a file system — a
    // reader sees either the whole old catalogue or the whole new one.
    let script = replace_script(video_dir, &target, &temp, &expected);
    let out = match conn.exec(&script).await {
        Ok(out) => out,
        Err(e) => {
            // The script removes the staged file on every way it can fail; this is for
            // when it never got to run or to say how it went. Whether the replacement
            // happened is unknown here, and the error says only that it did not report.
            let _ = sftp.remove_file(temp).await;
            return Err(ManifestIoError::Ssh(e));
        }
    };

    let verdict = verdict(&out.stdout);
    if verdict == "REPLACED" && out.ok() {
        tracing::info!(
            generation = manifest.generation,
            media = manifest.media.len(),
            "library catalogue written"
        );
        return Ok(());
    }
    if verdict.starts_with("CONFLICT") {
        // Somebody else's catalogue stands. Which generation it carries is read again
        // for the message — not under the lock, so it may already be later still; an
        // unreadable one is not a reason to hide that the write was refused.
        let current = match read(conn, video_dir).await {
            Ok(m) => m.generation,
            Err(_) => u64::MAX,
        };
        return Err(ManifestIoError::Conflict {
            base: base_generation,
            current,
        });
    }
    if verdict.starts_with("LOCK_TIMEOUT") {
        return Err(ManifestIoError::Busy);
    }
    // The staged file is the script's to remove on every failure it reports; one it did
    // not report (a reply that makes no sense) is cleaned up here as well.
    let _ = sftp.remove_file(temp).await;
    Err(ManifestIoError::Ssh(crate::ssh::SshError::Exec(format!(
        "the catalogue was not replaced: exit {:?}, said {:?}{}",
        out.exit_code,
        verdict,
        if out.stderr.trim().is_empty() {
            String::new()
        } else {
            format!(", complained {:?}", out.stderr.trim())
        }
    ))))
}

/// The one step on the server that decides whether the staged catalogue goes in.
///
/// Every way out prints a verdict as its last line, and every way out but success removes
/// the staged file — nothing is ever reported as replaced without the `mv` having said so.
/// The lock is held on descriptor 9 for the life of the block; it is given back when the
/// shell exits, whatever it exits with (and by the kernel if the shell is killed).
///
/// The block is a compound command on purpose: a failed redirection on `exec 9<` would end
/// a non-interactive `sh` on the spot, before it could remove the staged file; on `{ }`
/// it only skips the block.
fn replace_script(video_dir: &str, target: &str, temp: &str, expected: &str) -> String {
    let dir = shell_quote(video_dir);
    let target = shell_quote(target);
    let temp = shell_quote(temp);
    format!(
        "{{\n\
         flock -x -w {LOCK_WAIT_SECS} -E {LOCK_TIMEOUT_EXIT} 9; rc=$?\n\
         if [ $rc = {LOCK_TIMEOUT_EXIT} ]; then rm -f -- {temp}; echo LOCK_TIMEOUT; exit 4; fi\n\
         if [ $rc != 0 ]; then rm -f -- {temp}; echo \"FAIL_LOCK $rc\"; exit 4; fi\n\
         if [ -e {target} ]; then\n\
         sum=$(sha256sum < {target}) || {{ rm -f -- {temp}; echo FAIL_SUM; exit 5; }}\n\
         sum=${{sum%% *}}\n\
         else sum={ABSENT}; fi\n\
         if [ \"$sum\" != '{expected}' ]; then rm -f -- {temp}; echo CONFLICT; exit 3; fi\n\
         mv -f -- {temp} {target} || {{ rm -f -- {temp}; echo FAIL_REPLACE; exit 6; }}\n\
         echo REPLACED\n\
         exit 0\n\
         }} 9< {dir}\n\
         rm -f -- {temp}\n\
         echo FAIL_LOCK_OPEN\n\
         exit 4\n"
    )
}

/// The last line the step printed — it ends by saying how it went.
fn verdict(stdout: &str) -> &str {
    stdout
        .lines()
        .rev()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("")
}
