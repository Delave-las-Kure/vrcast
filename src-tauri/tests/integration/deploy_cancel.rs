//! T609 phase B, T613, T614 — cancelling a deployment on a real server with real apt and dpkg.
//!
//! What phase A measured (`deploy_cancel_measure.rs`) is what these hold the code to:
//!
//! - (a) a cancel during `dpkg --unpack` answers `Cancelled` only once dpkg is done — at that
//!   moment no process carrying the run's mark is alive, no apt/dpkg is running and
//!   `dpkg --audit` is empty — and a repeat **at once** goes to the end (phase A: the old code
//!   answered in 2.4 s, dpkg ran 17.8 s more, and the repeat died on `Could not get lock`);
//! - (b) the SSH undo timer — the one process a run leaves behind on purpose — survives a
//!   cancel that confirms nothing of the run is left;
//! - (c) dpkg left interrupted (killed mid-unpack, and on a serving server mid-configure of
//!   fail2ban) is finished by the repeat itself, with nobody's hands on the server, and the
//!   serving answers;
//! - (d) T613: Packages over a Caddy keyring that is already there; T614: a failed apt
//!   install is reported in apt's own words.
//!
//! Needs Docker and the Ubuntu archive. The harness fetches the packages ahead with
//! `Acquire::Retries=10` (archive.ubuntu.com answered 503 on part of most batch downloads on
//! 2026-09-23); a failure with "Failed to fetch … 503" is the network, not the code.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::future::BoxFuture;
use vrcast_studio_lib::commands::error::ErrorCode;
use vrcast_studio_lib::domain::deploy_steps::{Status, StepId};
use vrcast_studio_lib::domain::dns_verdict::{Ipv6Choice, ServerAddresses};
use vrcast_studio_lib::server::deploy::{
    self, fail2ban, machine, packages, Context, DeployError, Proofs, RunMark, RUN_VAR,
};
use vrcast_studio_lib::ssh::keygen;
use vrcast_studio_lib::store::db::Db;
use vrcast_studio_lib::tasks::engine::TaskContext;

use super::deploy_cancel_measure::{
    configuring, connect, has_line, inside, launch_marked, packages_script, prewarm, repeat_once,
    steps_for_a_container, unpacking, wait_until, DOMAIN, DPKG_STATE, PS, SERVING, STOP,
};
use super::deploy_clean::{by_password, key_works, no_second_try, password_refused, VIDEO_DIR};
use super::deploy_fixture::{DeployTarget, Flavour};

/// Every live process whose environment carries the run's mark, by `/proc` alone — the
/// independent witness, not the code under test's own scan.
fn marked_alive(name: &str) -> String {
    inside(
        name,
        &format!(
            "for f in /proc/[0-9]*/environ; do p=${{f#/proc/}}; p=${{p%/environ}}
  [ \"$p\" = \"$$\" ] && continue
  if tr '\\0' '\\n' < \"$f\" 2>/dev/null | grep -q '^{RUN_VAR}='; then
    echo \"$p $(tr '\\0' ' ' < /proc/$p/cmdline 2>/dev/null | cut -c1-120)\"
  fi
done"
        ),
    )
}

fn apt_or_dpkg_running(ps: &str) -> bool {
    ps.lines().any(|l| {
        (l.contains("apt-get") || l.contains("/usr/bin/dpkg") || l.contains("/var/lib/dpkg/info"))
            && !l.contains("sleep 300")
    })
}

fn http_code(name: &str) -> String {
    let said = inside(name, SERVING);
    said.lines()
        .find_map(|l| l.strip_prefix("http: "))
        .unwrap_or("?")
        .trim()
        .to_owned()
}

/// What the moment of `Cancelled` looked like.
struct AtCancelled {
    code: Option<ErrorCode>,
    after_cancel: Duration,
    ps: String,
    marked: String,
    audit: String,
}

/// Run the deployment through the task runner as production does, cancel it when `phase`
/// is seen in the process list, and record the server the moment the runner answers.
async fn run_and_cancel_when(
    target: &DeployTarget,
    made: &keygen::MadeKey,
    phase: fn(&str) -> bool,
    what: &str,
) -> AtCancelled {
    let name = target.container_name().to_owned();
    let conn = connect(target, made).await;
    let facts = machine::look(&conn).await.expect("no machine facts");
    let key_proof =
        || -> BoxFuture<'_, bool> { Box::pin(key_works(target, &made.private_openssh)) };
    let password_proof = || -> BoxFuture<'_, bool> { Box::pin(password_refused(target)) };
    let ctx = Context {
        conn: &conn,
        domain: DOMAIN,
        video_dir: VIDEO_DIR,
        ipv6: Ipv6Choice::Keep,
        server: ServerAddresses { v4: None, v6: None },
        public_key: made.public_openssh.clone(),
        machine: facts,
        already_ours: false,
        replace_caddyfile: false,
        run: RunMark::fresh(),
        proofs: Proofs {
            key_works: &key_proof,
            password_refused: &password_proof,
        },
    };
    let steps = steps_for_a_container();
    let task = TaskContext::detached(Arc::new(Db::open_in_memory().unwrap()));
    let token = task.cancel_token();
    let finished = Arc::new(AtomicBool::new(false));
    let cancelled_at: Arc<std::sync::Mutex<Option<Instant>>> = Arc::default();

    let run = {
        let finished = finished.clone();
        let cancelled_at = cancelled_at.clone();
        let name = name.clone();
        let task = &task;
        let ctx = &ctx;
        let steps = &steps;
        async move {
            let r = vrcast_studio_lib::tasks::deploy::run(
                ctx,
                steps,
                task,
                &mut |_| {},
                &no_second_try,
            )
            .await;
            // The very moment the runner says it is over.
            let at = Instant::now();
            finished.store(true, Ordering::SeqCst);
            let ps = inside(&name, PS);
            let marked = marked_alive(&name);
            let audit = inside(&name, "dpkg --audit 2>&1");
            let after_cancel = cancelled_at
                .lock()
                .unwrap()
                .map(|c| at.duration_since(c))
                .unwrap_or_default();
            AtCancelled {
                code: r.err().map(|e| e.code),
                after_cancel,
                ps,
                marked,
                audit,
            }
        }
    };
    let trigger = {
        let name = name.clone();
        let finished = finished.clone();
        let what = what.to_owned();
        async move {
            let found = tokio::task::spawn_blocking(move || {
                wait_until(&name, phase, Duration::from_secs(900), Some(&finished))
            })
            .await
            .unwrap();
            match found {
                Some((_, ps)) => {
                    println!("=== {what} seen; cancelling. Running:\n{ps}");
                    *cancelled_at.lock().unwrap() = Some(Instant::now());
                    token.cancel();
                }
                None => println!("=== {what} was never seen; not cancelling"),
            }
        }
    };
    let (at, ()) = tokio::join!(run, trigger);
    conn.close().await;
    println!(
        "RUNNER ANSWERED {:?}, {:.1}s after the cancel\n--- apt/dpkg then:\n{}\n--- marked then:\n{}\n--- dpkg --audit then:\n{}",
        at.code,
        at.after_cancel.as_secs_f64(),
        at.ps,
        at.marked,
        at.audit
    );
    at
}

/// Repeat until a run ends for a reason that is not the archive (at most three runs).
async fn repeat_to_the_end(target: &DeployTarget, made: &keygen::MadeKey, what: &str) {
    for attempt in 1..=3 {
        let (done, text) = repeat_once(target, made).await;
        println!("[{what}, run {attempt}] {text}");
        if done {
            return;
        }
        assert!(
            super::deploy_cancel_measure::looks_like_network(&text) && attempt < 3,
            "{what}: the repeat did not reach the end, and not for a network reason: {text}"
        );
        println!("[{what}] that was the archive, not the code — again");
    }
}

/// (a) and (b), on one machine: a cancel during dpkg, then a cancel right after the SSH undo
/// timer was armed, then a repeat to the end.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_cancel_waits_for_dpkg_leaves_nothing_of_the_run_and_spares_the_undo_timer() {
    let target = DeployTarget::start(Flavour::Clean).expect("the bare container would not come up");
    let name = target.container_name().to_owned();
    let made = keygen::make("vrcast-studio: T609").expect("no key");
    prewarm(&name, "fail2ban unattended-upgrades");

    // (a) Cancelled during dpkg's unpack.
    let first = run_and_cancel_when(&target, &made, unpacking, "dpkg --unpack").await;
    assert_eq!(first.code, Some(ErrorCode::TaskCancelled));
    assert!(
        !apt_or_dpkg_running(&first.ps),
        "`Cancelled` came while apt/dpkg were still running on the server — constitution III:\n{}",
        first.ps
    );
    assert!(
        first.marked.trim().is_empty(),
        "at `Cancelled` processes carrying the run's mark were alive:\n{}",
        first.marked
    );
    assert!(
        first.audit.trim().is_empty(),
        "the cancel left dpkg with unfinished work:\n{}",
        first.audit
    );

    // (b) Cancelled right after the SSH undo timer was armed. The repeat starts at once —
    // phase A's `Could not get lock` would show up here.
    let armed = |ps: &str| has_line(ps, &["sleep 300"]);
    let second = run_and_cancel_when(&target, &made, armed, "the SSH undo timer").await;
    assert_eq!(second.code, Some(ErrorCode::TaskCancelled));
    assert!(
        second.marked.trim().is_empty(),
        "at `Cancelled` processes carrying the run's mark were alive:\n{}",
        second.marked
    );
    assert!(
        has_line(&second.ps, &["sleep 300"]),
        "the SSH undo timer did not survive the cancel — the server has lost the net that \
         gives it its old way in back:\n{}",
        second.ps
    );
    assert!(
        !apt_or_dpkg_running(&second.ps),
        "apt/dpkg running at `Cancelled`:\n{}",
        second.ps
    );

    // And at once, the repeat, to the end.
    repeat_to_the_end(&target, &made, "repeat after two cancels").await;
    let code = http_code(&name);
    assert!(
        code != "000" && code != "?",
        "the serving does not answer after the repeat (http {code})\n{}",
        inside(&name, SERVING)
    );
    println!("{}", inside(&name, DPKG_STATE));
}

/// (c) dpkg left interrupted — killed mid-unpack on a bare machine, then mid-configure of
/// fail2ban on the serving one — is finished by the repeat itself.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_repeat_finishes_what_an_interrupted_dpkg_left_undone() {
    let target = DeployTarget::start(Flavour::Clean).expect("the bare container would not come up");
    let name = target.container_name().to_owned();
    let made = keygen::make("vrcast-studio: T609").expect("no key");
    prewarm(&name, "fail2ban unattended-upgrades");

    launch_marked(&name, &packages_script());
    wait_until(&name, unpacking, Duration::from_secs(900), None)
        .expect("the unpack phase was never seen");
    println!("{}", inside(&name, STOP));
    let audit = inside(&name, "dpkg --audit 2>&1");
    assert!(
        !audit.trim().is_empty(),
        "the kill did not leave dpkg interrupted — this test checks nothing: {audit}"
    );
    println!(
        "--- after the kill in unpack:\n{}",
        inside(&name, DPKG_STATE)
    );
    repeat_to_the_end(&target, &made, "repeat after a kill in unpack").await;
    let code = http_code(&name);
    assert!(
        code != "000" && code != "?",
        "no serving after the repeat (http {code})"
    );
    assert!(inside(&name, "dpkg --audit 2>&1").trim().is_empty());

    // The same on a serving server, for fail2ban, killed while configuring.
    println!(
        "{}",
        inside(
            &name,
            "systemctl stop fail2ban; export DEBIAN_FRONTEND=noninteractive; \
             apt-get purge -y -qq fail2ban >/dev/null 2>&1; \
             apt-get autoremove --purge -y -qq >/dev/null 2>&1; \
             dpkg -l fail2ban 2>&1 | tail -n 1"
        )
    );
    prewarm(&name, "fail2ban");
    launch_marked(
        &name,
        "set -e\nDEBIAN_FRONTEND=noninteractive apt-get -o Acquire::Retries=10 install -y -qq fail2ban\necho done",
    );
    wait_until(&name, configuring, Duration::from_secs(600), None)
        .expect("the configure phase was never seen");
    println!("{}", inside(&name, STOP));
    let audit = inside(&name, "dpkg --audit 2>&1");
    assert!(
        !audit.trim().is_empty(),
        "the kill did not leave fail2ban's install interrupted — this checks nothing: {audit}"
    );
    let during = http_code(&name);
    println!("--- serving right after the kill: http {during}");
    repeat_to_the_end(
        &target,
        &made,
        "repeat after a kill in fail2ban's configure",
    )
    .await;
    assert!(inside(&name, "dpkg --audit 2>&1").trim().is_empty());
    let guarding = inside(
        &name,
        "systemctl is-active fail2ban; fail2ban-client status sshd >/dev/null 2>&1 && echo guarding",
    );
    assert!(
        guarding.contains("guarding"),
        "fail2ban is not guarding after the repeat: {guarding}"
    );
    let code = http_code(&name);
    assert!(
        code != "000" && code != "?",
        "no serving after the repeat (http {code})"
    );
}

/// Run single steps on a bare machine, by password. A blocking step's failure ends the run
/// as an `Err`; a non-blocking one's (Fail2ban) is only recorded — so the failures recorded
/// along the way are handed back as well.
async fn run_steps(
    target: &DeployTarget,
    made: &keygen::MadeKey,
    only: &[StepId],
) -> (Result<(), DeployError>, Vec<(StepId, String)>) {
    let conn = by_password(target).await;
    let facts = machine::look(&conn).await.expect("no machine facts");
    let key_proof =
        || -> BoxFuture<'_, bool> { Box::pin(key_works(target, &made.private_openssh)) };
    let password_proof = || -> BoxFuture<'_, bool> { Box::pin(password_refused(target)) };
    let ctx = Context {
        conn: &conn,
        domain: DOMAIN,
        video_dir: VIDEO_DIR,
        ipv6: Ipv6Choice::Keep,
        server: ServerAddresses { v4: None, v6: None },
        public_key: made.public_openssh.clone(),
        machine: facts,
        already_ours: false,
        replace_caddyfile: false,
        run: RunMark::fresh(),
        proofs: Proofs {
            key_works: &key_proof,
            password_refused: &password_proof,
        },
    };
    let steps: Vec<_> = deploy::all()
        .into_iter()
        .filter(|s| only.contains(&s.id))
        .collect();
    let never = || false;
    let mut failures = Vec::new();
    let outcome = deploy::run(&ctx, &steps, &never, &mut |s| {
        if let Status::Failed { detail } = &s.status {
            failures.push((s.id, detail.clone()));
        }
    })
    .await;
    conn.close().await;
    (outcome.map(|_| ()), failures)
}

/// (d) T613: Packages over a Caddy keyring that is already there. T614: a failed apt
/// install is reported in apt's words, not as what the step tripped over next.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn packages_go_over_an_existing_keyring_and_a_failed_install_says_what_apt_said() {
    let target = DeployTarget::start(Flavour::Clean).expect("the bare container would not come up");
    let name = target.container_name().to_owned();
    let made = keygen::make("vrcast-studio: T613").expect("no key");
    prewarm(&name, "");

    // What an interrupted run leaves: the keyring written, Caddy not installed.
    inside(
        &name,
        &format!(
            "mkdir -p /usr/share/keyrings && printf 'left by an earlier run' > {}",
            packages::KEYRING
        ),
    );
    let mut outcome = Err(DeployError::Cancelled);
    for attempt in 1..=3 {
        outcome = run_steps(&target, &made, &[StepId::Packages]).await.0;
        match &outcome {
            Err(e)
                if super::deploy_cancel_measure::looks_like_network(&e.to_string())
                    && attempt < 3 =>
            {
                println!("[packages {attempt}] the archive: {e} — again");
            }
            _ => break,
        }
    }
    if let Err(e) = &outcome {
        panic!("Packages failed over an existing keyring (T613): {e}");
    }
    assert!(
        inside(&name, "command -v caddy").contains("caddy"),
        "Caddy is not installed after Packages"
    );

    // T614: fail2ban made impossible to install — the package lists emptied, so apt answers
    // "Unable to locate package". The step must fail on the install, with apt's own
    // complaint — not thirty seconds later, on a jail that cannot exist.
    inside(&name, "rm -rf /var/lib/apt/lists/* && echo emptied");
    let t0 = Instant::now();
    let (outcome, failures) = run_steps(&target, &made, &[StepId::Fail2ban]).await;
    let took = t0.elapsed();
    println!(
        "T614: {outcome:?}, failures {failures:?} ({:.1}s)",
        took.as_secs_f64()
    );
    let [(StepId::Fail2ban, text)] = failures.as_slice() else {
        panic!("fail2ban could not be installed and the step did not fail: {failures:?}");
    };
    assert!(
        text.contains(fail2ban::PACKAGE) && text.contains("E:") && text.contains("apt failed"),
        "the failure does not carry apt's own words: {text}"
    );
    assert!(
        !text.contains("not guarding"),
        "the failure reports the consequence instead of the cause: {text}"
    );
    assert!(
        took < Duration::from_secs(25),
        "the step waited for a jail that could not exist before failing ({took:?})"
    );
}
