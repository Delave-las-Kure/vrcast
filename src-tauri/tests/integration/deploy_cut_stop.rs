//! T615 — a deployment whose connection breaks in the middle of dpkg answers `Failed` only
//! once dpkg is done on the server and nothing of the run is left, and a repeat at once goes
//! to the end.
//!
//! **The cut is `deploy_network_cut.rs`'s**: `docker network disconnect -f`, the TCP link
//! goes silent, and russh's keepalive closes the channel ~90–120 s later. That is longer than
//! the Packages step's dpkg takes in a container, so with the real packages `Failed` would
//! come after dpkg's end whatever the code did — a test that cannot fail. So the dpkg here is
//! made long on purpose: a local package whose `preinst` sleeps [`SLOW_S`], installed through
//! the production `Context::apt` (the interrupted-dpkg repair first, `apt-get … install`,
//! apt's own words on failure) by a step of the test's own, run by the production
//! `tasks::deploy::run`. dpkg is then still unpacking when the run's own connection is found
//! dead; the old code wrote `Failed` right there.
//!
//! `stop_again` is production's shape without the gate: a fresh connection (the network is
//! put back first — the "server is reachable again" moment), `server::marked::stop_confirmed`
//! with the patience the run hands it, closed.
//!
//! Needs Docker; the repeat needs the Ubuntu archive (fetched ahead with retries; a failure
//! with "Failed to fetch … 503" is the network, not the code).

use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures::future::BoxFuture;
use vrcast_studio_lib::commands::error::ErrorCode;
use vrcast_studio_lib::domain::deploy_steps::{Change, Checked, StepId};
use vrcast_studio_lib::domain::dns_verdict::{Ipv6Choice, ServerAddresses};
use vrcast_studio_lib::domain::marked::Stopped;
use vrcast_studio_lib::server::deploy::{
    self, machine, Context, Proofs, RunMark, Step, APT_GET, RUN_VAR, TEMP_SUFFIX,
};
use vrcast_studio_lib::server::marked::Patience;
use vrcast_studio_lib::ssh::{fingerprint, keygen, Connection, Credentials};
use vrcast_studio_lib::store::db::Db;
use vrcast_studio_lib::tasks::engine::TaskContext;

use super::deploy_cancel_measure::{inside, prewarm, repeat_once, wait_until, DOMAIN, PS};
use super::deploy_clean::{address, by_password, key_works, password_refused, VIDEO_DIR};
use super::deploy_fixture::{DeployTarget, Flavour, ROOT_PASSWORD};

/// How long the package's `preinst` sleeps inside `dpkg --unpack`.
///
/// Longer than the keepalive takes to find the cut (~120 s, measured by T560) by a margin
/// that leaves dpkg still running when the run's own error arrives, and when the first stop
/// through a fresh connection is asked.
const SLOW_S: u64 = 180;

const SLOW_DEB: &str = "/root/vrcast-slow.deb";
const SLOW_PKG: &str = "vrcast-slow";

/// Where an earlier, interrupted `put_file` would have left its half-written copy — one of
/// the paths a run really writes (`TEMP_PLACES`): since T622 only those are tidied.
const LEFT_BEHIND: &str = "/etc/sysctl.d/99-vrcast-net.conf";

fn make_slow_package(name: &str) {
    let said = inside(
        name,
        &format!(
            "set -e
rm -rf /root/slow && mkdir -p /root/slow/DEBIAN
printf 'Package: {SLOW_PKG}\\nVersion: 1.0\\nArchitecture: all\\nMaintainer: t615\\nDescription: T615 slow unpack\\n' > /root/slow/DEBIAN/control
printf '#!/bin/sh\\necho preinst sleeping\\nsleep {SLOW_S}\\necho preinst done\\nexit 0\\n' > /root/slow/DEBIAN/preinst
chmod 755 /root/slow/DEBIAN/preinst
dpkg-deb --build /root/slow {SLOW_DEB} >/dev/null
echo built"
        ),
    );
    assert!(
        said.contains("built"),
        "the slow package was not built: {said}"
    );
}

fn slow_step<'a>() -> Step<Context<'a>> {
    fn changes(_: &Context<'_>) -> Vec<Change> {
        Vec::new()
    }
    fn check<'b>(ctx: &'b Context<'_>) -> BoxFuture<'b, deploy::Result<Checked>> {
        Box::pin(async move {
            let there = ctx
                .asks(&format!(
                    "dpkg-query -W -f='${{Status}}' {SLOW_PKG} 2>/dev/null | grep -q 'install ok installed' && echo yes || echo no"
                ))
                .await?;
            Ok(if there {
                Checked::Applied
            } else {
                Checked::NotApplied
            })
        })
    }
    fn apply<'b>(ctx: &'b Context<'_>) -> BoxFuture<'b, deploy::Result<()>> {
        Box::pin(async move {
            ctx.apt(
                StepId::Packages,
                &format!("{APT_GET} install -y -qq {SLOW_DEB}"),
            )
            .await
        })
    }
    Step {
        id: StepId::Packages,
        changes,
        check,
        apply,
    }
}

fn docker_network(verb: &str, name: &str) -> String {
    let mut args = vec!["network", verb];
    if verb == "disconnect" {
        args.push("-f");
    }
    args.extend_from_slice(&["bridge", name]);
    let out = Command::new("docker")
        .args(&args)
        .output()
        .expect("docker would not run");
    format!(
        "{} {}{}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// Puts the network back however the test ends.
struct Reconnect(String);
impl Drop for Reconnect {
    fn drop(&mut self) {
        let _ = docker_network("connect", &self.0);
    }
}

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
        l.contains("apt-get")
            || l.contains("/usr/bin/dpkg")
            || l.contains("dpkg --")
            || l.contains("/var/lib/dpkg/")
    })
}

fn preinst_sleeping(ps: &str) -> bool {
    ps.lines()
        .any(|l| l.contains("preinst") && l.contains("/var/lib/dpkg/"))
}

pub(super) async fn fresh_connection(target: &DeployTarget) -> Result<Connection, String> {
    let a = address(target).await;
    let fp = fingerprint::probe(&a)
        .await
        .map_err(|e| format!("no fingerprint: {e}"))?;
    Connection::connect(
        a,
        "root",
        Credentials::Password(ROOT_PASSWORD.to_owned()),
        &fp,
    )
    .await
    .map_err(|e| format!("no connection: {e}"))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_cut_during_dpkg_is_failed_only_after_dpkg_ended_and_a_repeat_goes_to_the_end() {
    let target = DeployTarget::start(Flavour::Clean).expect("the bare container would not come up");
    let name = target.container_name().to_owned();
    let made = keygen::make("vrcast-studio: T615").expect("no key");
    // For the repeat with the real steps afterwards; the slow package itself needs no archive.
    prewarm(&name, "fail2ban unattended-upgrades");
    make_slow_package(&name);
    inside(
        &name,
        &format!("mkdir -p /etc/sysctl.d && echo half > {LEFT_BEHIND}{TEMP_SUFFIX}"),
    );

    let conn = by_password(&target).await;
    let facts = machine::look(&conn).await.expect("no machine facts");
    let key_proof =
        || -> BoxFuture<'_, bool> { Box::pin(key_works(&target, &made.private_openssh)) };
    let password_proof = || -> BoxFuture<'_, bool> { Box::pin(password_refused(&target)) };
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
    let steps = vec![slow_step()];
    let task = TaskContext::detached(Arc::new(Db::open_in_memory().unwrap()));

    let t0 = Instant::now();
    let finished = Arc::new(AtomicBool::new(false));
    let cut_at: Arc<Mutex<Option<Duration>>> = Arc::default();
    let dpkg_last_seen: Arc<Mutex<Option<Duration>>> = Arc::default();
    let attempts = AtomicUsize::new(0);
    let attempt_log: Mutex<Vec<String>> = Mutex::default();
    let _reconnect = Reconnect(name.clone());

    let stop_again = |mark: String, patience: Patience| -> BoxFuture<'_, Result<Stopped, String>> {
        let n = attempts.fetch_add(1, Ordering::SeqCst) + 1;
        let target = &target;
        let name = name.clone();
        let log = &attempt_log;
        Box::pin(async move {
            // The server comes back: the network is put back before the first fresh try.
            let back = docker_network("connect", &name);
            let started = t0.elapsed();
            let result = match fresh_connection(target).await {
                Err(e) => Err(e),
                Ok(fresh) => {
                    let r = vrcast_studio_lib::server::marked::stop_confirmed(
                        &fresh, RUN_VAR, &mark, patience,
                    )
                    .await
                    .map_err(|p| p.to_string());
                    fresh.close().await;
                    r
                }
            };
            log.lock().unwrap().push(format!(
                "stop_again #{n} at {:.1}s ({patience:?}; network: {}) -> {result:?} at {:.1}s",
                started.as_secs_f64(),
                back.trim(),
                t0.elapsed().as_secs_f64()
            ));
            result
        })
    };

    let run = async {
        let r =
            vrcast_studio_lib::tasks::deploy::run(&ctx, &steps, &task, &mut |_| {}, &stop_again)
                .await;
        let at = t0.elapsed();
        finished.store(true, Ordering::SeqCst);
        (
            r,
            at,
            inside(&name, PS),
            marked_alive(&name),
            inside(&name, "dpkg --audit 2>&1"),
        )
    };
    let cutter = {
        let name = name.clone();
        let finished = finished.clone();
        let cut_at = cut_at.clone();
        let dpkg_last_seen = dpkg_last_seen.clone();
        async move {
            tokio::task::spawn_blocking(move || {
                let seen = wait_until(
                    &name,
                    preinst_sleeping,
                    Duration::from_secs(300),
                    Some(&finished),
                );
                let Some((_, ps)) = seen else {
                    println!("=== the slow preinst was never seen; not cutting");
                    return;
                };
                println!("=== dpkg is unpacking; cutting the network. Running:\n{ps}");
                let said = docker_network("disconnect", &name);
                *cut_at.lock().unwrap() = Some(t0.elapsed());
                println!("disconnect: {}", said.trim());
                // Independent witness of when apt/dpkg end on the server (docker exec needs
                // no network).
                while !finished.load(Ordering::SeqCst) || t0.elapsed() < Duration::from_secs(1) {
                    let ps = inside(&name, PS);
                    if apt_or_dpkg_running(&ps) {
                        *dpkg_last_seen.lock().unwrap() = Some(t0.elapsed());
                    }
                    std::thread::sleep(Duration::from_millis(500));
                }
            })
            .await
            .unwrap();
        }
    };
    let ((outcome, at, ps, marked, audit), ()) = tokio::join!(run, cutter);
    conn.close().await;
    drop(_reconnect);

    let cut = cut_at.lock().unwrap().expect("the network was never cut");
    let last_dpkg = *dpkg_last_seen.lock().unwrap();
    println!(
        "CUT at {:.1}s; RUNNER ANSWERED at {:.1}s: {outcome:?}\n  apt/dpkg last seen at {:?}\n{}\n--- apt/dpkg then:\n{ps}\n--- marked then:\n{marked}\n--- dpkg --audit then:\n{audit}",
        cut.as_secs_f64(),
        at.as_secs_f64(),
        last_dpkg.map(|d| d.as_secs_f64()),
        attempt_log.lock().unwrap().join("\n"),
    );

    let error = outcome.expect_err("a run cut off in the middle of dpkg came back Ok");
    assert_ne!(
        error.code,
        ErrorCode::TaskCancelled,
        "a broken connection was read as a cancel: {error:?}"
    );
    let cause = format!("{:?}", error);
    assert!(
        !cause.contains("gate") && !cause.contains("stop"),
        "the failure carries the stop's words, not the step's: {cause}"
    );
    assert!(
        attempts.load(Ordering::SeqCst) >= 1,
        "the stop through the dead connection was taken as confirmed"
    );
    assert!(
        !apt_or_dpkg_running(&ps),
        "`Failed` came while apt/dpkg were still running on the server:\n{ps}"
    );
    assert!(
        marked.trim().is_empty(),
        "at `Failed` processes carrying the run's mark were alive:\n{marked}"
    );
    assert!(
        audit.trim().is_empty(),
        "dpkg was left with unfinished work:\n{audit}"
    );
    if let Some(last) = last_dpkg {
        assert!(
            last <= at,
            "apt/dpkg were seen at {last:?}, after `Failed` at {at:?}"
        );
    }
    assert!(
        at > cut + Duration::from_secs(SLOW_S / 2),
        "the runner answered {:.1}s after the cut — dpkg's preinst sleeps {SLOW_S}s, so it \
         cannot have waited for it",
        (at - cut).as_secs_f64()
    );
    let installed = inside(
        &name,
        &format!("dpkg-query -W -f='${{Status}}' {SLOW_PKG} 2>&1"),
    );
    println!("--- {SLOW_PKG}: {installed}");

    // And at once, the repeat with the real steps, to the end.
    for attempt in 1..=3 {
        let (done, text) = repeat_once(&target, &made).await;
        println!("[repeat {attempt}] {text}");
        if done {
            break;
        }
        assert!(
            super::deploy_cancel_measure::looks_like_network(&text) && attempt < 3,
            "the repeat did not reach the end, and not for a network reason: {text}"
        );
    }
    let left = inside(
        &name,
        &format!("ls {LEFT_BEHIND}{TEMP_SUFFIX} 2>&1; ls /etc/*{TEMP_SUFFIX} 2>/dev/null | wc -l"),
    );
    assert!(
        left.contains("No such file"),
        "what an interrupted write left was not tidied by the next run: {left}"
    );
    assert!(inside(&name, "dpkg --audit 2>&1").trim().is_empty());
}
