//! T629 — a run whose copy of the settings could not be made changes nothing but its mark
//! (QA-20 №5).
//!
//! `back_up` used to run `[ -e "$f" ] && cp -a … || true` for each file and never read the
//! block's exit status: a file that was there and would not copy was skipped as if absent,
//! `latest` was moved onto the incomplete copy, `Ok` came back — and the run went on to replace
//! the very file whose copy was missing. Now the run stops before its first step.
//!
//! Through the production runner (`tasks::deploy::run`), on a bare container, twice:
//!
//! 1. the backup directory is read-only (a read-only tmpfs over `/etc/vrcast/backup`): not even
//!    the copy's own directory can be made;
//! 2. the backup directory has room for the small files and not for a big `jail.local`: the
//!    copy starts, `cp` of that file fails, and an earlier `latest` must still be where it was.
//!
//! Each time: `DEPLOY_STEP_FAILED` with `DEPLOY_STOPPED_AFTER {done: 0}`, no step reported,
//! and the machine as it was — no Caddy, no `vrcast` user, no state file, sshd still taking
//! passwords, the owner's files untouched — except the T617 mark, `/var/lib/vrcast`.
//!
//! Needs Docker (nothing is installed: the run stops before `Packages`).

use std::sync::Arc;

use futures::future::BoxFuture;
use vrcast_studio_lib::domain::deploy_steps::StepId;
use vrcast_studio_lib::domain::dns_verdict::{Ipv6Choice, ServerAddresses};
use vrcast_studio_lib::domain::wording::DetailCode;
use vrcast_studio_lib::error::ErrorCode;
use vrcast_studio_lib::server::deploy::{self, machine, Context, Proofs, RunMark};
use vrcast_studio_lib::ssh::keygen;
use vrcast_studio_lib::store::db::Db;
use vrcast_studio_lib::tasks::engine::TaskContext;

use super::deploy_clean::{by_password, key_works, no_second_try, password_refused, VIDEO_DIR};
use super::deploy_fixture::{DeployTarget, Flavour};

const JAIL: &str = "the owner wrote this jail";
const CADDY: &str = ":8080 {\n\trespond \"the owner's serving\"\n}\n";

/// What the run must not have changed, asked of the machine.
const UNCHANGED: &str = "\
dpkg -s caddy >/dev/null 2>&1 && echo caddy-installed
id vrcast >/dev/null 2>&1 && echo user-made
test -e /etc/vrcast/state.json && echo state-written
sshd -T | grep -qx 'passwordauthentication yes' || echo password-off
ufw status | head -n 1 | grep -q 'Status: active' && echo firewall-on
grep -qx 'the owner wrote this jail' /etc/fail2ban/jail.local || echo jail-touched
grep -q \"the owner's serving\" /etc/caddy/Caddyfile || echo caddyfile-touched
test -d /var/lib/vrcast || echo no-mark
true";

async fn run_once(target: &DeployTarget) -> vrcast_studio_lib::error::AppError {
    let made = keygen::make("vrcast-studio: T629").expect("no key was made");
    let conn = by_password(target).await;
    let facts = machine::look(&conn).await.expect("no machine facts");
    let key_proof =
        || -> BoxFuture<'_, bool> { Box::pin(key_works(target, &made.private_openssh)) };
    let password_proof = || -> BoxFuture<'_, bool> { Box::pin(password_refused(target)) };
    let ctx = Context {
        conn: &conn,
        domain: "vrcast-container.invalid",
        video_dir: VIDEO_DIR,
        ipv6: Ipv6Choice::Keep,
        server: ServerAddresses { v4: None, v6: None },
        public_key: made.public_openssh.clone(),
        machine: facts,
        already_ours: false,
        // The person agreed to replace the Caddyfile "because a copy is kept" — the promise
        // this check is about.
        replace_caddyfile: true,
        run: RunMark::fresh(),
        proofs: Proofs {
            key_works: &key_proof,
            password_refused: &password_proof,
        },
    };
    let steps: Vec<_> = deploy::all()
        .into_iter()
        .filter(|s| !matches!(s.id, StepId::DnsCheck | StepId::Verify))
        .collect();
    let task = TaskContext::detached(Arc::new(Db::open_in_memory().unwrap()));
    let mut reported = 0usize;
    let outcome = vrcast_studio_lib::tasks::deploy::run(
        &ctx,
        &steps,
        &task,
        &mut |settled| reported = settled.len(),
        &no_second_try,
    )
    .await;
    conn.close().await;
    assert_eq!(reported, 0, "a step was run after the copy failed");
    outcome.expect_err("the run went on without its copy of the settings")
}

fn assert_refused_before_any_step(error: &vrcast_studio_lib::error::AppError) {
    assert_eq!(error.code, ErrorCode::DeployStepFailed, "{error:?}");
    assert!(
        error
            .details
            .iter()
            .any(|d| d.key == DetailCode::DeployStoppedAfter
                && d.params.get("done").and_then(|v| v.as_u64()) == Some(0)),
        "not said as stopped before any step: {error:?}"
    );
    assert!(
        error
            .cause
            .as_deref()
            .is_some_and(|c| c.contains("could not be copied aside")),
        "{error:?}"
    );
}

fn assert_unchanged(target: &DeployTarget) {
    let seen = target
        .exec_inside(UNCHANGED)
        .expect("the machine would not answer");
    assert!(
        seen.trim().is_empty(),
        "the run changed the machine without a copy of its settings: {seen}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_run_without_its_copy_changes_nothing_but_its_mark() {
    let target = DeployTarget::start(Flavour::Clean).expect("the bare container would not come up");
    target
        .exec_inside(&format!(
            "mkdir -p /etc/fail2ban /etc/caddy /etc/vrcast/backup \
             && printf '%s\\n' '{JAIL}' > /etc/fail2ban/jail.local \
             && printf '%s' \"{}\" > /etc/caddy/Caddyfile",
            CADDY.replace('"', "\\\"")
        ))
        .expect("could not seed the owner's files");

    // 1. Nowhere to write the copy at all.
    target
        .exec_inside("mount -t tmpfs -o ro,size=1m tmpfs /etc/vrcast/backup")
        .expect("could not make the backup directory read-only");
    let error = run_once(&target).await;
    println!("read-only: {error:?}");
    assert_refused_before_any_step(&error);
    assert!(
        error
            .cause
            .as_deref()
            .is_some_and(|c| c.contains("Read-only")),
        "the server's own words are not in the failure: {error:?}"
    );
    assert_unchanged(&target);

    // 2. Room for some files and not for the big one: the copy starts and a `cp` fails. An
    //    earlier copy is what `latest` points at, and must still be.
    target
        .exec_inside(
            "umount /etc/vrcast/backup \
             && mount -t tmpfs -o size=64k tmpfs /etc/vrcast/backup \
             && mkdir /etc/vrcast/backup/earlier \
             && ln -s /etc/vrcast/backup/earlier /etc/vrcast/backup/latest \
             && head -c 1048576 /dev/zero | tr '\\0' '#' >> /etc/fail2ban/jail.local",
        )
        .expect("could not make the backup directory small");
    // `jail.local` is now the owner's line and a megabyte after it — too big for the room left.
    let size_before = target
        .exec_inside("stat -c %s /etc/fail2ban/jail.local")
        .expect("no size");
    let error = run_once(&target).await;
    println!("no room: {error:?}");
    assert_refused_before_any_step(&error);
    assert!(
        error
            .cause
            .as_deref()
            .is_some_and(|c| c.contains("/etc/fail2ban/jail.local would not be copied")),
        "the file that would not copy is not named: {error:?}"
    );
    let after = target
        .exec_inside(
            "readlink /etc/vrcast/backup/latest; ls /etc/vrcast/backup | sort | tr '\\n' ' '; echo; \
             stat -c %s /etc/fail2ban/jail.local; head -n 1 /etc/fail2ban/jail.local",
        )
        .expect("the machine would not answer");
    let lines: Vec<&str> = after.lines().collect();
    assert_eq!(
        lines.first().copied(),
        Some("/etc/vrcast/backup/earlier"),
        "latest was moved off the earlier copy: {after}"
    );
    assert_eq!(
        lines.get(1).map(|l| l.trim()),
        Some("earlier latest"),
        "the half-made copy was left behind: {after}"
    );
    assert_eq!(lines.get(2).copied(), Some(size_before.trim()), "{after}");
    assert!(lines.get(3).is_some_and(|l| l.starts_with(JAIL)), "{after}");
    assert_unchanged(&target);
}
