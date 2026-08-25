//! A key for the tests: made on the spot, never committed to the repository.
//!
//! Shared by the unit and the integration tests deliberately. Creating it used to live in
//! the integration fixture while the unit tests used a ready file — and on a developer's
//! machine that worked, because the file was already there from an earlier run. In
//! continuous integration the unit tests run as a job of their own, where the integration
//! ones had never been, and the key was not there. Caught by the very first run on GitHub on
//! 2026-08-25.
//!
//! Why the key is not kept in the repository — see `tests/fixtures/README.md`.

use std::path::PathBuf;
use std::process::Command;

/// The passphrase. Not a secret: the key is made right here and opens nothing anywhere.
pub const PASSPHRASE: &str = "test-passphrase-1234";

pub fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// The private key.
pub fn key_path() -> PathBuf {
    fixtures_dir().join("encrypted_ed25519.key")
}

/// The public half — inside the container image's build directory.
pub fn public_key_path() -> PathBuf {
    fixtures_dir().join("docker/encrypted_ed25519.key.pub")
}

/// Whether both halves of the key are there.
///
/// Both specifically: with only one half `ssh-keygen` refuses to overwrite the other, and
/// such a key cannot be logged in with anyway.
fn ready() -> bool {
    key_path().exists() && public_key_path().exists()
}

/// Create the key if it is not there yet.
///
/// It is called from several tests at once, and that is the main subtlety. The tests run in
/// several threads; without a lock two of them decide at the same time that the key is
/// missing, both wipe the half-made files and both call `ssh-keygen`. The second runs into a
/// file that already exists, `ssh-keygen` asks about overwriting, reads end-of-input and
/// exits with an error without saying a word to the error stream. It looks like "the test
/// key was not created:" with nothing after the colon. Caught by a run on Windows on
/// 2026-08-25; before that the race simply never came up, by luck.
pub fn ensure() -> Result<PathBuf, String> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    // A poisoned lock is no reason to refuse the work: there is no state under it that a
    // failed thread could have spoilt.
    let _held = LOCK.lock().unwrap_or_else(|e| e.into_inner());

    if ready() {
        return Ok(key_path());
    }

    // The half-made state is cleared: with one of the two files present, ssh-keygen
    // refuses to overwrite and the tests stop with an unintelligible error.
    let generated_pub = fixtures_dir().join("encrypted_ed25519.key.pub");
    let _ = std::fs::remove_file(key_path());
    let _ = std::fs::remove_file(&generated_pub);
    let _ = std::fs::remove_file(public_key_path());

    let out = Command::new("ssh-keygen")
        .args([
            "-t",
            "ed25519",
            "-q",
            "-N",
            PASSPHRASE,
            "-C",
            "vrcast-studio: a throwaway key for the tests",
            "-f",
        ])
        .arg(key_path())
        .output()
        .map_err(|e| {
            format!(
                "could not run ssh-keygen: {e}. It is needed to create the test key; on \
                 Windows it comes with OpenSSH, on Linux with openssh-client"
            )
        })?;

    if !out.status.success() {
        // The lock holds only this process. A neighbouring `cargo` run may have created
        // the key while we were calling `ssh-keygen` — and then all is well: what is needed
        // is a key that is there, not our authorship of it.
        if ready() {
            return Ok(key_path());
        }
        return Err(format!(
            "the test key was not created (code {}):\n{}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        ));
    }

    if let Some(parent) = public_key_path().parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::copy(&generated_pub, public_key_path()).map_err(|e| {
        format!("could not put the key's public half into the build directory: {e}")
    })?;

    Ok(key_path())
}
