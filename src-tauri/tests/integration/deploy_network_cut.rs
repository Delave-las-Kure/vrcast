//! T560 — a deployment interrupted by a REAL network failure, not a voluntary cancel.
//!
//! **What `deploy_resume.rs` already proves, and what it cannot.** Its own header says so
//! plainly: "Прерывание сделано отменой, а не убийством: движок спрашивает «отменили?»
//! перед каждым шагом" — the engine's `cancelled()` closure returns `true` on cue, which is
//! the path a person clicking "cancel" takes. `spec.md`'s Edge Cases (FR-015-adjacent line
//! about SC-015) names a different failure on purpose: "Развёртывание прервано **обрывом
//! связи** на середине" — the connection itself dies, mid-command, with nobody asking for
//! it. That is a different code path end to end: no `cancelled()` check catches it, because
//! nothing calls it — the command already in flight has to fail (or the channel has to die)
//! on its own for `deploy::run` to ever come back at all.
//!
//! **What was verified by hand before this test was written, because the answer was not
//! obvious.** A `docker network disconnect -f` mid-`Packages` step (`apt-get update &&
//! apt-get install`, the one step here with substantial real network I/O — measured at
//! roughly 40 seconds on an unaffected run) does NOT fail fast. The TCP connection goes
//! silent rather than resetting: a raw read on the same socket did not return within 60
//! seconds by itself. What actually ends it is `russh`'s own keepalive
//! (`keepalive_interval: 30s, keepalive_max: 3` in `ssh::fingerprint::client_config`) —
//! after roughly 90-120 seconds of silence the channel closes from underneath the running
//! command, `channel.wait()`'s message loop ends with nothing left to read, and
//! `Context::ran` sees an empty, exit-code-less `CommandOutput` that is NOT itself an
//! `SshError` — the apply reads it as "ran and said nothing", and reports
//! `DeployError::Step` with an empty detail, not `DeployError::Ssh`. Reproduced twice by
//! hand, agreeing to within 20 milliseconds both times (~120.14s, ~120.16s from the same
//! disconnect instant) — this is a real, deterministic mechanism, not test flakiness.
//!
//! **What this test asserts, and deliberately does not.** It does not hard-code which step
//! was interrupted or what shape `DeployError` takes — machine speed, package-mirror
//! latency and even which step is mid-flight when the cut lands can all vary between this
//! project's own dev machine and whatever runs its continuous integration, and a test
//! pinned to `StepId::Packages` specifically would be testing today's timing rather than
//! the property SC-015 actually promises. What is asserted, and is true regardless of which
//! step got caught:
//!
//! 1. `deploy::run` returns within a bounded time — not instantly (that would mean the cut
//!    was a no-op) and not never (a hang here is the worst possible outcome: the person is
//!    left staring at a spinner with no way to know the deployment is dead).
//! 2. The result is `Err`, and specifically NOT `DeployError::Cancelled` — nobody's
//!    `cancelled()` closure ever returns `true` in this test, so a `Cancelled` result here
//!    would mean the failure was misclassified as a voluntary cancel, which is exactly the
//!    distinction this test exists to keep real.
//! 3. Once the network is restored, a fresh connection and a fresh `deploy::run` against
//!    the SAME container — no manual intervention on the machine at all — finishes with
//!    every step `Applied` or cleanly `Skipped`, none `Failed` or left `NotApplied`. This is
//!    SC-015's own wording: "довод[ится] до конца повторным запуском... без ручной правки
//!    сервера."

use std::time::{Duration, Instant};

use futures::future::BoxFuture;
use vrcast_studio_lib::domain::deploy_steps::{Status, StepId};
use vrcast_studio_lib::domain::dns_verdict::{Ipv6Choice, ServerAddresses};
use vrcast_studio_lib::server::deploy::{self, machine, Context, DeployError, Proofs};
use vrcast_studio_lib::ssh::keygen;

use super::deploy_clean::{by_password, key_works, password_refused, VIDEO_DIR};
use super::deploy_fixture::{DeployTarget, Flavour};

/// How long after `deploy::run` starts the network is cut.
///
/// `Swap` (the step before `Packages`) settles in microseconds inside a container — it is
/// always `NotPossibleHere` there — so by three seconds in, `Packages` (`apt-get update`,
/// `apt-get install`, a `curl` for Caddy's own repository) is realistically still running:
/// none of that is instant even on a fast mirror, and apt's own overhead alone routinely
/// costs more than three seconds. Not tied to the `watch` callback because it only fires
/// when a step SETTLES, not when one starts — there is no earlier, more precise hook to
/// catch "a network-bound step is now in flight" than a short, generous wall-clock delay.
const CUT_AFTER: Duration = Duration::from_secs(3);

/// How long `deploy::run` is given to notice the network is gone and give up.
///
/// Measured by hand at ~120.15 seconds, twice, agreeing to within 20 milliseconds — this
/// leaves a wide margin (double) for a slower machine while still failing the test, rather
/// than hanging the test suite forever, if the underlying mechanism (`russh`'s keepalive)
/// stops detecting the cut at all.
const RUN_MUST_RETURN_WITHIN: Duration = Duration::from_secs(240);

/// Build a fresh `Context` against `target`, reusing the one key made for the whole test.
fn context_for<'a>(
    target: &'a DeployTarget,
    conn: &'a vrcast_studio_lib::ssh::Connection,
    made: &'a keygen::MadeKey,
    machine: machine::Machine,
    key_proof: &'a (dyn Fn() -> BoxFuture<'a, bool> + Sync),
    password_proof: &'a (dyn Fn() -> BoxFuture<'a, bool> + Sync),
) -> Context<'a> {
    let _ = target; // kept as a parameter for symmetry with the fixtures this mirrors
    Context {
        conn,
        domain: "vrcast-container.invalid",
        video_dir: VIDEO_DIR,
        ipv6: Ipv6Choice::Keep,
        server: ServerAddresses { v4: None, v6: None },
        public_key: made.public_openssh.clone(),
        machine,
        already_ours: false,
        proofs: Proofs {
            key_works: key_proof,
            password_refused: password_proof,
        },
    }
}

#[tokio::test]
async fn a_real_network_cut_mid_deployment_fails_cleanly_and_a_fresh_run_finishes_it() {
    let target = DeployTarget::start(Flavour::Clean).expect("the bare container would not come up");
    let container_name = target.container_name().to_owned();
    let made = keygen::make("vrcast-studio: T560 network cut").expect("no key was made");

    let conn = by_password(&target).await;
    let facts = machine::look(&conn).await.expect("no machine facts");
    let key_proof =
        || -> BoxFuture<'_, bool> { Box::pin(key_works(&target, &made.private_openssh)) };
    let password_proof = || -> BoxFuture<'_, bool> { Box::pin(password_refused(&target)) };
    let ctx = context_for(&target, &conn, &made, facts, &key_proof, &password_proof);

    let steps: Vec<_> = deploy::all()
        .into_iter()
        .filter(|s| !matches!(s.id, StepId::DnsCheck | StepId::Verify))
        .collect();

    // Real network I/O has to be interrupted, not a request to stop: this closure never
    // returns `true`. If the run below came back `Cancelled`, that would mean the failure
    // was being misread as a voluntary cancel — the one thing this test exists to catch.
    let never = || false;

    let mut watched: Vec<(StepId, Status)> = Vec::new();
    let run_started = Instant::now();
    let mut record_step = |planned: &vrcast_studio_lib::domain::deploy_steps::PlannedStep| {
        watched.push((planned.id, planned.status.clone()));
    };
    let run_fut = deploy::run(&ctx, &steps, &never, &mut record_step);

    let cutter = async {
        tokio::time::sleep(CUT_AFTER).await;
        let out = std::process::Command::new("docker")
            .args(["network", "disconnect", "-f", "bridge", &container_name])
            .output();
        assert!(
            matches!(&out, Ok(o) if o.status.success()),
            "could not cut the container's network — the fixture itself is broken, and \
             nothing below would be testing what this file claims to: {out:?}"
        );
    };

    let (run_result, ()) = tokio::join!(
        tokio::time::timeout(RUN_MUST_RETURN_WITHIN, run_fut),
        cutter
    );
    let elapsed = run_started.elapsed();

    // Restore connectivity before anything else, successful or not: a later assertion
    // failing here must not leave the container permanently cut off from a network that a
    // later run in the same suite might need it to have (and `DeployTarget::drop` itself
    // needs no network to force-remove the container, but leaving litter with a severed
    // network is still worth avoiding).
    let reconnected = std::process::Command::new("docker")
        .args(["network", "connect", "bridge", &container_name])
        .output();
    assert!(
        matches!(&reconnected, Ok(o) if o.status.success()),
        "the network could not be reconnected after the cut, so nothing past this point can \
         be trusted: {reconnected:?}"
    );

    // 1. Bounded, not instant and not infinite.
    let timed_out = run_result.is_err();
    assert!(
        !timed_out,
        "deploy::run() never returned within {RUN_MUST_RETURN_WITHIN:?} of a real network \
         cut — a hang here is worse than a clean failure: the person is left watching a \
         spinner with no way to know the deployment is dead. Steps settled before the \
         timeout: {watched:?}"
    );
    assert!(
        elapsed > CUT_AFTER,
        "deploy::run() returned in {elapsed:?}, before the cut at {CUT_AFTER:?} even landed \
         — the cut was not actually exercised by this run"
    );

    // 2. A real Err, and specifically not the voluntary-cancel shape.
    let outcome = run_result.expect("checked above: not a timeout");
    let error = outcome.expect_err(
        "deploy::run() returned Ok despite a real network cut mid-run — a step must have \
         been reported done without the server actually being asked, which is exactly the \
         six-month mistake `ssh_hardening.rs`'s own header describes",
    );
    assert!(
        !matches!(error, DeployError::Cancelled),
        "a real network failure came back as DeployError::Cancelled — nobody's cancelled() \
         closure ever returned true in this test, so this would mean a network death is \
         being misclassified as a voluntary cancel: {error:?}"
    );
    eprintln!(
        "network cut acknowledged after {elapsed:?} as: {error} (steps: {watched:?})"
    );

    // 3. The resume: a fresh connection, a fresh run, no hand-fixing of the server.
    tokio::time::sleep(Duration::from_secs(1)).await;
    let conn2 = by_password(&target).await;
    let facts2 = machine::look(&conn2)
        .await
        .expect("the machine could not be read again after the network was restored");
    let key_proof2 =
        || -> BoxFuture<'_, bool> { Box::pin(key_works(&target, &made.private_openssh)) };
    let password_proof2 = || -> BoxFuture<'_, bool> { Box::pin(password_refused(&target)) };
    let ctx2 = context_for(&target, &conn2, &made, facts2, &key_proof2, &password_proof2);

    let resumed = deploy::run(&ctx2, &steps, &never, &mut |_| {})
        .await
        .expect(
            "the deployment could not be finished by a fresh run after the network came \
             back — SC-015 promises exactly this for any interrupted step, and a real \
             network death is one of the cases it has to cover, not only a voluntary cancel",
        );

    for step in &resumed {
        assert!(
            !matches!(step.status, Status::NotApplied | Status::Failed { .. }),
            "{:?} came back as {:?} on the resume — the interrupted deployment was not \
             actually finished off",
            step.id,
            step.status
        );
    }

    // And what the cut-off step eventually needed is really on the server, asked of the
    // machine rather than trusted from the run's own report — the same shape of check
    // `deploy_clean.rs` and `deploy_resume.rs` use for the same reason.
    let seen = target
        .exec_inside(
            "command -v ffmpeg >/dev/null 2>&1 && echo ffmpeg
command -v caddy >/dev/null 2>&1 && echo caddy
[ \"$(systemctl is-active caddy)\" = active ] && echo serving
sshd -T | grep -qx 'passwordauthentication no' && echo password-off
test -f /etc/vrcast/state.json && echo state",
        )
        .expect("the machine would not answer after the resumed deployment");
    for expected in ["ffmpeg", "caddy", "serving", "password-off", "state"] {
        assert!(
            seen.contains(expected),
            "{expected} is missing after a deployment interrupted by a real network cut was \
             finished off by a fresh run: {seen}"
        );
    }
}
