//! T629 — the copy made before a run's first change is read, not assumed (QA-20 №5).
//!
//! `back_up` used to run `[ -e "$f" ] && cp -a … || true` per file and never read the block's
//! exit status: a file that was there and would not copy was skipped as if absent, `latest` was
//! moved onto the incomplete copy, and the run went on to replace the file. What is checked here
//! is the reading of the server's answer (`backup_outcome`) and the shape of the block; the real
//! refusal against a container is `tests/integration/deploy_backup_fails.rs`.

use vrcast_studio_lib::server::deploy::DeployError;
use vrcast_studio_lib::server::upgrade::{back_up_script, backup_outcome, owned_files, LATEST};
use vrcast_studio_lib::ssh::CommandOutput;

fn said(exit_code: Option<u32>, stdout: &str, stderr: &str) -> CommandOutput {
    CommandOutput {
        exit_code,
        stdout: stdout.to_owned(),
        stderr: stderr.to_owned(),
    }
}

const INTO: &str = "/etc/vrcast/backup/20260928T000000Z";

/// Where it went, every file answered — the first copied, the rest absent — then `done`.
fn whole() -> String {
    let mut out = format!("into {INTO}\n");
    for (i, f) in owned_files().iter().enumerate() {
        out.push_str(if i == 0 { "copied " } else { "absent " });
        out.push_str(f);
        out.push('\n');
    }
    out.push_str("done\n");
    out
}

fn detail_of<T: std::fmt::Debug>(r: Result<T, DeployError>) -> String {
    match r {
        Err(DeployError::NotBackedUp { detail }) => detail,
        other => panic!("not refused as NotBackedUp: {other:?}"),
    }
}

#[test]
fn a_whole_copy_is_accepted_and_says_where_it_went() {
    assert_eq!(
        backup_outcome(&said(Some(0), &whole(), "")).ok().as_deref(),
        Some(INTO)
    );
    // A second run in the same second copies beside the first, not into it.
    let beside = whole().replace(&format!("into {INTO}"), &format!("into {INTO}-2"));
    assert_eq!(
        backup_outcome(&said(Some(0), &beside, "")).ok(),
        Some(format!("{INTO}-2"))
    );
}

#[test]
fn a_copy_that_does_not_say_where_it_went_is_refused() {
    let nowhere = whole().replace(&format!("into {INTO}\n"), "");
    let detail = detail_of(backup_outcome(&said(Some(0), &nowhere, "")));
    assert!(detail.contains("no directory"), "{detail}");
    // Nor one that names a place outside the backups.
    let elsewhere = whole().replace(&format!("into {INTO}"), "into /tmp/x");
    detail_of(backup_outcome(&said(Some(0), &elsewhere, "")));
}

#[test]
fn a_file_that_would_not_copy_is_refused_and_named() {
    let caddyfile = "/etc/caddy/Caddyfile";
    let r = backup_outcome(&said(
        Some(1),
        &format!("into {INTO}\nfailed {caddyfile}\n"),
        "cp: cannot create regular file '/etc/vrcast/backup/x/Caddyfile': Permission denied",
    ));
    let detail = detail_of(r);
    assert!(detail.contains(caddyfile), "{detail}");
    assert!(detail.contains("Permission denied"), "{detail}");
    assert!(detail.contains("exit 1"), "{detail}");
}

#[test]
fn a_block_that_failed_before_any_file_is_refused() {
    // `mkdir` of the copy's directory refused: nothing on standard output at all.
    let detail = detail_of(backup_outcome(&said(
        Some(1),
        "",
        "mkdir: cannot create directory '/etc/vrcast/backup': Read-only file system",
    )));
    assert!(detail.contains("no directory"), "{detail}");
    assert!(detail.contains("Read-only"), "{detail}");
}

#[test]
fn done_is_not_enough_without_exit_zero() {
    // `ln` of `latest` failed after every file: set -e stops before `done` in reality, but the
    // exit status decides whatever was printed.
    detail_of(backup_outcome(&said(Some(1), &whole(), "ln: failed")));
    detail_of(backup_outcome(&said(None, &whole(), "")));
}

#[test]
fn exit_zero_is_not_enough_without_done_or_an_answer_for_every_file() {
    let no_done = whole().replace("done\n", "");
    detail_of(backup_outcome(&said(Some(0), &no_done, "")));

    let last = owned_files().last().unwrap().clone();
    let one_missing = whole().replace(&format!("absent {last}\n"), "");
    let detail = detail_of(backup_outcome(&said(Some(0), &one_missing, "")));
    assert!(detail.contains(&last), "{detail}");

    // A `failed` line with exit 0 — whatever else was said — is not a copy either.
    let failed_too = format!("failed /etc/fstab\n{}", whole());
    detail_of(backup_outcome(&said(Some(0), &failed_too, "")));
}

#[test]
fn the_block_tells_absent_from_failed_and_moves_latest_last() {
    let script = back_up_script("/etc/vrcast/backup/20260928T000000Z");
    assert!(
        !script.contains("|| true"),
        "a failed copy is silenced again:\n{script}"
    );
    assert!(script.starts_with("set -e"), "{script}");
    for needle in ["absent $f", "copied $f", "failed $f", "exit 1", "into $d"] {
        assert!(script.contains(needle), "{needle} is missing:\n{script}");
    }
    // `latest` is moved once, after the loop over the files — never before a file was copied.
    let ln = script.find("ln -sfn").expect("latest is not moved");
    assert!(script[ln..].contains(LATEST));
    assert!(
        script[..ln].trim_end().ends_with("done"),
        "latest is not moved right after the loop:\n{script}"
    );
    assert!(script.trim_end().ends_with("echo done"), "{script}");
}
