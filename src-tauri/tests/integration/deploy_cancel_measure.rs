//! T609 phase A — measurement: what cancelling a deployment does to apt/dpkg on the server
//! today, and whether apt/dpkg can be killed mid-work without breaking the server (I) and
//! without breaking the repeat (V).
//!
//! **A measurement, not an assertion** (precedent: T517, `lane_overlap_bench.rs`). Every
//! scenario prints what it saw; nothing here decides anything. Run one at a time, with Docker,
//! and never two Docker runs at once:
//!
//! ```text
//! cargo test --features integration --test integration -- --ignored --nocapture --test-threads=1 t609_
//! ```
//!
//! The harness's own apt calls carry `-o Acquire::Retries=10` (archive.ubuntu.com answered 503
//! on part of a batch download on 2026-09-23), and the packages are fetched ahead
//! (`--download-only`) where the scenario is about dpkg, so the killed install and the repeat
//! do not depend on the archive. The repeats use the **current production code of the steps,
//! unchanged**, which has no retries — a repeat that fails with "Failed to fetch … 503" is a
//! network failure, not a consequence of the kill, and is retried (up to three times) and
//! reported as such.
//!
//! The kill itself is T605's stop reduced: the killed script runs in a session and group of
//! its own with a mark in its environment; TERM to the group and to every marked process,
//! 5 s, KILL, 5 s.
//!
//! Results are in the report of T609 (2026-09-23) and summarised here.
//!
//! **Measured 2026-09-23, Docker Desktop/WSL2, `vrcast-test-clean:1`, one run per scenario**
//! (the archive answered 503 on the first prewarm of most runs; every result below comes from
//! a run that reached dpkg or finished without a network error, unless said otherwise):
//!
//! | scenario | stop | dpkg after | repeat, current code |
//! |---|---|---|---|
//! | today: cancel while `dpkg --unpack` runs (`t609_cancel_today_during_packages`) | runner returned `TaskCancelled` at once; apt-get + dpkg still alive at that moment and 1 s after `conn.close()`; gone 17.8 s later (apt finished the install) | clean (`audit` empty, 0 not-ii) | Ok, all Applied/Skipped, Caddy 308 |
//! | today, repeat at once (`t609_cancel_today_then_repeat_at_once`) | same | — | **Failed** `E: Could not get lock /var/lib/dpkg/lock-frontend. It is held by process 487 (apt-get)`; after the orphan ended (16.8 s) Ok |
//! | kill in `apt-get update` | TERM, 105 ms | clean | Ok |
//! | kill in the download (`/usr/lib/apt/methods/http`, no dpkg yet) | TERM, 106 ms | clean | Ok |
//! | kill in `dpkg --unpack` | TERM, 106 ms | 6 not-ii (`iU`×4, `iHR dbus-system-bus-common`, `it libc-bin`), 15 files in `updates/` | **Failed**: `E: dpkg was interrupted, you must manually run 'dpkg --configure -a' to correct the problem.` Caddy never installed |
//! | kill in `dpkg --configure --pending` | TERM, 106 ms | 230 `iU`, 71 files in `updates/` | **Failed**, same message |
//! | same two, then `dpkg --configure -a && apt-get -f install` by hand (`t609_self_heal_*`) | — | 0 not-ii after 0.5 s (unpack) / 7.5 s (configure) | Ok, all Applied/Skipped, Caddy 308 |
//! | serving server, kill fail2ban's install in configure | TERM, 106 ms | 6 `iU` (fail2ban, python3-*) | Caddy stayed up (308/200) through the kill and the repeat; repeat **Failed** at Fail2ban: `the sshd jail is not guarding after 30s: unit=failed … Found no accessible config files for 'fail2ban' under /etc/fail2ban` — the step drops apt's own complaint (`ctx.ran(...)` result unused) |
//! | kill in `apt-get install caddy` (after the keyring was written) | TERM, 105 ms | clean | **Failed**: `gpg: cannot open '/dev/tty': No such device or address` — `gpg --dearmor -o` over an existing keyring asks to overwrite; independent of dpkg, any interruption after the keyring write does it |
//!
//! So killing apt in its download/update phase is safe (I and V hold); killing dpkg is not
//! (V fails with the current step code; with a `dpkg --configure -a` first it held here).

use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::future::BoxFuture;
use vrcast_studio_lib::domain::deploy_steps::{PlannedStep, Status, StepId};
use vrcast_studio_lib::domain::dns_verdict::{Ipv6Choice, ServerAddresses};
use vrcast_studio_lib::server::deploy::{self, machine, Context, Proofs};
use vrcast_studio_lib::ssh::{fingerprint, keygen, Connection, Credentials};
use vrcast_studio_lib::store::db::Db;
use vrcast_studio_lib::tasks::engine::TaskContext;

use super::deploy_clean::{
    address, by_password, key_works, no_second_try, password_refused, VIDEO_DIR,
};
use super::deploy_fixture::{DeployTarget, Flavour};

pub(super) const DOMAIN: &str = "vrcast-container.invalid";
pub(super) const MARK: &str = "VRCAST_T609";
pub(super) const LOG: &str = "/root/t609.log";

/// Run a script inside the container and hand back everything it said, whatever its exit
/// code — the measurement wants the complaint as much as the answer.
pub(super) fn inside(name: &str, script: &str) -> String {
    let out = Command::new("docker")
        .args(["exec", name, "bash", "-c", script])
        .output()
        .expect("docker exec would not run");
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// Put a file inside the container through stdin (no quoting games).
pub(super) fn put_inside(name: &str, path: &str, body: &str) {
    let mut child = Command::new("docker")
        .args(["exec", "-i", name, "bash", "-c", &format!("cat > {path}")])
        .stdin(Stdio::piped())
        .spawn()
        .expect("docker exec -i would not start");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(body.as_bytes())
        .unwrap();
    assert!(child.wait().unwrap().success(), "could not write {path}");
}

/// Every apt/dpkg-related process, with its group and session.
pub(super) const PS: &str = "ps -eo pid,pgid,sid,stat,etimes,args --no-headers \
    | grep -E 'apt-get|dpkg|/methods/|/var/lib/dpkg/info|unattended|gpg|curl|sleep 300' \
    | grep -v -E 'grep -E|ps -eo' | cut -c1-220";

/// The state dpkg is left in.
pub(super) const DPKG_STATE: &str =
    "echo '--- dpkg --audit:'; dpkg --audit 2>&1; echo \"audit rc=$?\"
echo '--- dpkg -l (not ii):'; dpkg -l 2>/dev/null | grep -Ev '^(ii|Desired|\\||\\+)' | head -n 40
echo \"--- not-ii count: $(dpkg -l 2>/dev/null | grep -Ev '^(ii|Desired|\\||\\+)' | wc -l)\"
echo \"--- /var/lib/dpkg/updates: $(ls /var/lib/dpkg/updates | wc -l) files\"
echo \"--- caddy keyring: $(ls -la /usr/share/keyrings/caddy-stable-archive-keyring.gpg 2>&1)\"
echo \"--- caddy list: $(ls -la /etc/apt/sources.list.d/caddy-stable.list 2>&1)\"";

/// Serving: is Caddy running and does it answer on port 80 (the container has no certificate
/// for the made-up domain, so an HTTP answer — a redirect — is what "answers" means here).
pub(super) const SERVING: &str = "echo \"caddy: $(systemctl is-active caddy 2>&1)\"
echo \"http: $(curl -s -o /dev/null -w '%{http_code}' --max-time 5 -H 'Host: vrcast-container.invalid' http://127.0.0.1/ 2>&1)\"
echo \"admin: $(curl -s -o /dev/null -w '%{http_code}' --max-time 5 http://127.0.0.1:2019/config/ 2>&1)\"";

/// The harness's copy of the Packages step's apply — the production text exactly, except for
/// the retries on apt's own downloads.
pub(super) fn packages_script() -> String {
    let names = deploy::packages::FROM_APT.join(" ");
    format!(
        "set -e
export DEBIAN_FRONTEND=noninteractive
R='-o Acquire::Retries=10'
apt-get $R update -qq
apt-get $R install -y -qq {names}
if ! command -v caddy >/dev/null; then
  curl -1sLf https://dl.cloudsmith.io/public/caddy/stable/gpg.key | gpg --dearmor -o /usr/share/keyrings/caddy-stable-archive-keyring.gpg
  curl -1sLf https://dl.cloudsmith.io/public/caddy/stable/debian.deb.txt > /etc/apt/sources.list.d/caddy-stable.list
  apt-get $R update -qq
  apt-get $R install -y -qq caddy
fi
echo done"
    )
}

/// Fetch ahead what the Packages step installs from the distribution, so the killed install
/// and the repeat reach dpkg without depending on the archive. Retried: the archive is flaky.
pub(super) fn prewarm(name: &str, extra: &str) {
    let names = deploy::packages::FROM_APT.join(" ");
    // The image's docker-clean hook deletes downloaded archives after every dpkg run; for
    // a measurement about dpkg the cache has to survive one run to the next.
    inside(name, "rm -f /etc/apt/apt.conf.d/docker-clean");
    let t0 = Instant::now();
    for attempt in 1..=4 {
        let said = inside(
            name,
            &format!(
                "export DEBIAN_FRONTEND=noninteractive; R='-o Acquire::Retries=10'
apt-get $R update -qq 2>&1 && apt-get $R install --download-only -y -qq {names} {extra} 2>&1; echo \"rc=$?\""
            ),
        );
        if said.trim_end().ends_with("rc=0") {
            println!(
                "prewarm: {:.1}s ({attempt} attempt(s)), {} archives in the cache",
                t0.elapsed().as_secs_f64(),
                inside(name, "ls /var/cache/apt/archives/*.deb | wc -l").trim()
            );
            return;
        }
        println!("prewarm attempt {attempt} failed:\n{said}");
    }
    panic!("the archive would not give the packages in four attempts — an external failure");
}

/// Start a script in a session and group of its own, carrying the mark in its environment —
/// the shape phase B would give every remote command.
pub(super) fn launch_marked(name: &str, script: &str) {
    put_inside(name, "/root/t609-apply.sh", script);
    let out = Command::new("docker")
        .args([
            "exec",
            "-d",
            name,
            "bash",
            "-c",
            &format!("export {MARK}=run; setsid bash /root/t609-apply.sh > {LOG} 2>&1 < /dev/null"),
        ])
        .output()
        .unwrap();
    assert!(out.status.success(), "the marked script would not start");
}

/// TERM to every group holding a marked process and to each marked process, wait up to 5 s,
/// KILL whoever is left, wait up to 5 s — T605's `stop_script`, reduced. Prints what happened.
pub(super) const STOP: &str = r#"
scan() {
  M=()
  local d p st e; local -a f env
  for d in /proc/[0-9]*; do
    p=${d#/proc/}; [ "$p" = "$$" ] && continue
    read -r st 2>/dev/null < "$d/stat" || continue
    f=(${st##*) }); case "${f[0]}" in Z|X|x) continue ;; esac
    env=(); mapfile -d '' -t env 2>/dev/null < "$d/environ" || continue
    for e in "${env[@]}"; do
      if [ "$e" = "VRCAST_T609=run" ]; then
        M+=("$p:${f[2]}")
        case "$G" in *" ${f[2]} "*) ;; *) G+="${f[2]} " ;; esac
      fi
    done
  done
}
G=" "
sig() { local g x; for g in $G; do [ "$g" -gt 1 ] && kill -s "$1" -- "-$g" 2>/dev/null; done
        for x in "${M[@]}"; do kill -s "$1" "${x%%:*}" 2>/dev/null; done; }
t0=${EPOCHREALTIME//[.,]/}
ms() { local n=${EPOCHREALTIME//[.,]/}; echo $(( (n - t0) / 1000 )); }
scan; echo "marked before: ${#M[@]} processes, groups:$G"
[ ${#M[@]} -eq 0 ] && { echo 'STOP none'; exit 0; }
sig TERM
for i in $(seq 1 50); do sleep 0.1; scan; [ ${#M[@]} -eq 0 ] && { echo "STOP term $(ms)ms"; exit 0; }; done
echo "after TERM, 5 s, still: ${M[*]}"
for x in "${M[@]}"; do echo "  $(tr '\0' ' ' < /proc/${x%%:*}/cmdline 2>/dev/null | cut -c1-150)"; done
sig KILL
for i in $(seq 1 50); do sleep 0.1; scan; [ ${#M[@]} -eq 0 ] && { echo "STOP kill $(ms)ms"; exit 0; }; done
echo "STOP alive ${M[*]}"
"#;

pub(super) fn has_line(ps: &str, all: &[&str]) -> bool {
    ps.lines().any(|l| all.iter().all(|w| l.contains(w)))
}

fn updating(ps: &str) -> bool {
    has_line(ps, &["apt-get", "update"]) && has_line(ps, &["/methods/"])
}
fn downloading(ps: &str) -> bool {
    has_line(ps, &["apt-get", "install"])
        && has_line(ps, &["/methods/http"])
        && !has_line(ps, &["dpkg", "--status-fd"])
}
pub(super) fn unpacking(ps: &str) -> bool {
    has_line(ps, &["dpkg", "--unpack"])
}
pub(super) fn configuring(ps: &str) -> bool {
    has_line(ps, &["dpkg", "--configure"]) || has_line(ps, &["/var/lib/dpkg/info/", "configure"])
}
fn installing_caddy(ps: &str) -> bool {
    has_line(ps, &["apt-get", "install", "caddy"])
}
fn apt_or_dpkg(ps: &str) -> bool {
    has_line(ps, &["apt-get"]) || has_line(ps, &["dpkg"])
}

/// Poll until `wanted` holds of a `ps` snapshot. `None` when the patience ran out or `stop`
/// was raised.
pub(super) fn wait_until(
    name: &str,
    wanted: fn(&str) -> bool,
    patience: Duration,
    stop: Option<&AtomicBool>,
) -> Option<(Duration, String)> {
    let t0 = Instant::now();
    while t0.elapsed() < patience {
        if stop.is_some_and(|s| s.load(Ordering::SeqCst)) {
            return None;
        }
        let ps = inside(name, PS);
        if wanted(&ps) {
            return Some((t0.elapsed(), ps));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    println!(
        "gave up waiting after {:.0}s; log:\n{}",
        t0.elapsed().as_secs_f64(),
        inside(name, &format!("tail -n 20 {LOG} 2>/dev/null"))
    );
    None
}

pub(super) fn steps_for_a_container<'a>() -> Vec<deploy::Step<Context<'a>>> {
    deploy::all()
        .into_iter()
        .filter(|s| !matches!(s.id, StepId::DnsCheck | StepId::Verify))
        .collect()
}

pub(super) fn summary(steps: &[PlannedStep]) -> String {
    steps
        .iter()
        .map(|s| {
            let st = match &s.status {
                Status::Applied => String::from("Applied"),
                Status::Skipped { .. } => String::from("Skipped"),
                Status::NotApplied => String::from("NotApplied"),
                Status::Failed { detail } => format!("FAILED({detail})"),
            };
            format!("{:?}={st}", s.id)
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// A connection by the application's key if it already works, by password otherwise.
pub(super) async fn connect(target: &DeployTarget, made: &keygen::MadeKey) -> Connection {
    let a = address(target).await;
    let fp = fingerprint::probe(&a).await.expect("no fingerprint");
    if let Ok(conn) = Connection::connect(
        a,
        "root",
        Credentials::KeyText {
            openssh: made.private_openssh.clone(),
            passphrase: None,
        },
        &fp,
    )
    .await
    {
        return conn;
    }
    by_password(target).await
}

pub(super) fn looks_like_network(text: &str) -> bool {
    [
        "Failed to fetch",
        " 503 ",
        "Temporary failure resolving",
        "Could not connect",
        "Connection timed out",
        "Unable to fetch some archives",
        // The mirror mid-sync (seen 2026-09-24): an index whose hash does not match yet.
        "Hash Sum mismatch",
        "Some index files failed to download",
    ]
    .iter()
    .any(|w| text.contains(w))
}

/// One repeat of the deployment with the **current** step code, through the task runner, on
/// a fresh connection. Returns whether every step ended Applied/Skipped, and a description.
pub(super) async fn repeat_once(target: &DeployTarget, made: &keygen::MadeKey) -> (bool, String) {
    let conn = connect(target, made).await;
    let facts = machine::look(&conn).await.expect("no machine facts");
    let key_proof =
        || -> BoxFuture<'_, bool> { Box::pin(key_works(target, &made.private_openssh)) };
    let password_proof = || -> BoxFuture<'_, bool> { Box::pin(password_refused(target)) };
    let already_ours = target.has("/etc/vrcast/state.json").unwrap_or(false);
    let ctx = Context {
        conn: &conn,
        domain: DOMAIN,
        video_dir: VIDEO_DIR,
        ipv6: Ipv6Choice::Keep,
        server: ServerAddresses { v4: None, v6: None },
        public_key: made.public_openssh.clone(),
        machine: facts,
        already_ours,
        run: vrcast_studio_lib::server::deploy::RunMark::fresh(),
        proofs: Proofs {
            key_works: &key_proof,
            password_refused: &password_proof,
        },
    };
    let steps = steps_for_a_container();
    let task = TaskContext::detached(Arc::new(Db::open_in_memory().unwrap()));
    let t0 = Instant::now();
    let mut last: Vec<PlannedStep> = Vec::new();
    let outcome = vrcast_studio_lib::tasks::deploy::run(
        &ctx,
        &steps,
        &task,
        &mut |s| {
            last = s.to_vec();
        },
        &no_second_try,
    )
    .await;
    conn.close().await;
    let all_done = outcome.is_ok()
        && last
            .iter()
            .all(|s| matches!(s.status, Status::Applied | Status::Skipped { .. }));
    let text = format!(
        "REPEAT ({:.1}s, already_ours={already_ours}): {}\n  steps: {}",
        t0.elapsed().as_secs_f64(),
        match &outcome {
            Ok(_) => String::from("Ok"),
            Err(e) => format!("Err({e:?})"),
        },
        summary(&last)
    );
    (all_done, text)
}

/// Repeat until a run ends for a reason that is not the network (at most three runs).
async fn repeat(target: &DeployTarget, made: &keygen::MadeKey) {
    for attempt in 1..=3 {
        let (done, text) = repeat_once(target, made).await;
        println!("[repeat {attempt}] {text}");
        println!("[repeat {attempt}] all steps Applied/Skipped: {done}");
        if done || !looks_like_network(&text) {
            return;
        }
        println!("[repeat {attempt}] that was the network, not the kill — again");
    }
}

fn after_kill_report(name: &str) {
    println!("--- processes after the stop:\n{}", inside(name, PS));
    println!("--- script log:\n{}", inside(name, &format!("cat {LOG}")));
    println!("{}", inside(name, DPKG_STATE));
    println!("--- `apt-get -s install` after the kill (what a person would try):");
    println!(
        "{}",
        inside(
            name,
            &format!(
                "DEBIAN_FRONTEND=noninteractive apt-get -s install -y {} 2>&1 | grep -E '^E:|^W:|dpkg was interrupted|^Inst |^Conf ' | head -n 8",
                deploy::packages::FROM_APT.join(" ")
            )
        )
    );
}

/// Option 2 of the fork, measured without touching the step: after a kill in the dpkg phase,
/// the self-heal a step would have to run first — then the repeat with the current code.
async fn kill_then_self_heal(label: &str, phase: fn(&str) -> bool) {
    let target = DeployTarget::start(Flavour::Clean).expect("the bare container would not come up");
    let name = target.container_name().to_owned();
    let made = keygen::make("vrcast-studio: T609").expect("no key");

    prewarm(&name, "");
    launch_marked(&name, &packages_script());
    let Some((after, ps)) = wait_until(&name, phase, Duration::from_secs(900), None) else {
        println!("RESULT {label}: the phase was never seen");
        return;
    };
    println!(
        "=== {label}: seen {:.1}s after the start; running:\n{ps}",
        after.as_secs_f64()
    );
    println!("{}", inside(&name, STOP));
    println!(
        "{}",
        inside(
            &name,
            "echo \"not-ii after the kill: $(dpkg -l | grep -Ev '^(ii|Desired|\\||\\+)' | wc -l)\""
        )
    );
    let t = Instant::now();
    let healed = inside(
        &name,
        "export DEBIAN_FRONTEND=noninteractive
dpkg --configure -a >/tmp/heal1 2>&1; echo \"configure -a rc=$?\"
apt-get -o Acquire::Retries=10 -f install -y -qq >/tmp/heal2 2>&1; echo \"-f install rc=$?\"
grep -E '^E:|^W:|error' /tmp/heal1 /tmp/heal2 | head -n 10
echo \"not-ii after the heal: $(dpkg -l | grep -Ev '^(ii|Desired|\\||\\+)' | wc -l)\"
dpkg --audit; echo \"audit rc=$?\"",
    );
    println!(
        "--- self-heal ({:.1}s):\n{healed}",
        t.elapsed().as_secs_f64()
    );
    repeat(&target, &made).await;
    println!("--- after the repeat:\n{}", inside(&name, DPKG_STATE));
    println!("{}", inside(&name, SERVING));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "T609 measurement: needs Docker and the Ubuntu archive; run by hand"]
async fn t609_self_heal_after_unpack_kill() {
    kill_then_self_heal("dpkg --unpack", unpacking).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "T609 measurement: needs Docker and the Ubuntu archive; run by hand"]
async fn t609_self_heal_after_configure_kill() {
    kill_then_self_heal("dpkg --configure", configuring).await;
}

/// Today's cancel, and a person pressing "deploy" again at once: the cancelled task no longer
/// holds `DEPLOY_ALREADY_RUNNING`, and the orphaned apt on the server still holds dpkg's lock.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "T609 measurement: needs Docker and the Ubuntu archive; run by hand"]
async fn t609_cancel_today_then_repeat_at_once() {
    let target = DeployTarget::start(Flavour::Clean).expect("the bare container would not come up");
    let name = target.container_name().to_owned();
    let made = keygen::make("vrcast-studio: T609").expect("no key");
    prewarm(&name, "");

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
        run: vrcast_studio_lib::server::deploy::RunMark::fresh(),
        proofs: Proofs {
            key_works: &key_proof,
            password_refused: &password_proof,
        },
    };
    let steps = steps_for_a_container();
    let task = TaskContext::detached(Arc::new(Db::open_in_memory().unwrap()));
    let token = task.cancel_token();
    let finished = Arc::new(AtomicBool::new(false));
    let finished_run = finished.clone();
    let run = async {
        let r =
            vrcast_studio_lib::tasks::deploy::run(&ctx, &steps, &task, &mut |_| {}, &no_second_try)
                .await;
        finished_run.store(true, Ordering::SeqCst);
        r
    };
    let name2 = name.clone();
    let trigger = async move {
        let found = tokio::task::spawn_blocking(move || {
            wait_until(&name2, unpacking, Duration::from_secs(900), Some(&finished))
        })
        .await
        .unwrap();
        if found.is_some() {
            token.cancel();
        }
    };
    let (outcome, ()) = tokio::join!(run, trigger);
    println!(
        "RUNNER RETURNED: {:?}",
        outcome.as_ref().err().map(|e| format!("{e:?}"))
    );
    conn.close().await;
    println!("--- at the repeat:\n{}", inside(&name, PS));
    let (done, text) = repeat_once(&target, &made).await;
    println!("[immediate repeat] {text}\n[immediate repeat] all Applied/Skipped: {done}");
    // And once the orphan is gone.
    let t0 = Instant::now();
    while apt_or_dpkg(&inside(&name, PS)) && t0.elapsed() < Duration::from_secs(900) {
        std::thread::sleep(Duration::from_secs(1));
    }
    println!("--- orphan gone {:.1}s later", t0.elapsed().as_secs_f64());
    repeat(&target, &made).await;
    println!("{}", inside(&name, SERVING));
}

/// One kill scenario on a bare machine: start the Packages install marked, wait for the
/// phase, stop TERM→KILL, report, then repeat the deployment with the current code.
async fn kill_in_packages(label: &str, phase: fn(&str) -> bool, warm: bool) {
    let target = DeployTarget::start(Flavour::Clean).expect("the bare container would not come up");
    let name = target.container_name().to_owned();
    let made = keygen::make("vrcast-studio: T609").expect("no key");

    if warm {
        prewarm(&name, "");
    }
    if label == "download" {
        // Everything fetched ahead except the five largest archives: a download phase short
        // enough that the repeat does not hang on the flaky archive, long enough to be hit.
        println!(
            "{}",
            inside(
                &name,
                "cd /var/cache/apt/archives && ls -S *.deb | head -n 5 | tee /dev/stderr | xargs rm -f"
            )
        );
    }
    launch_marked(&name, &packages_script());
    let Some((after, ps)) = wait_until(&name, phase, Duration::from_secs(900), None) else {
        println!("RESULT {label}: the phase was never seen");
        return;
    };
    println!(
        "=== {label}: seen {:.1}s after the start; running:\n{ps}",
        after.as_secs_f64()
    );
    let t = Instant::now();
    println!("{}", inside(&name, STOP));
    println!("(stop took {:.1}s round trip)", t.elapsed().as_secs_f64());
    after_kill_report(&name);

    // The repeat, with the step code exactly as shipped.
    repeat(&target, &made).await;
    println!("--- after the repeat:\n{}", inside(&name, DPKG_STATE));
    println!("{}", inside(&name, SERVING));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "T609 measurement: needs Docker and the Ubuntu archive; run by hand"]
async fn t609_kill_during_apt_update() {
    kill_in_packages("apt-get update", updating, true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "T609 measurement: needs Docker and the Ubuntu archive; run by hand"]
async fn t609_kill_during_download() {
    kill_in_packages("download", downloading, true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "T609 measurement: needs Docker and the Ubuntu archive; run by hand"]
async fn t609_kill_during_dpkg_unpack() {
    kill_in_packages("dpkg --unpack", unpacking, true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "T609 measurement: needs Docker and the Ubuntu archive; run by hand"]
async fn t609_kill_during_dpkg_configure() {
    kill_in_packages("dpkg --configure", configuring, true).await;
}

/// Killed after the Caddy repository was added and before Caddy is installed: the repeat then
/// takes the `if ! command -v caddy` branch again, over a keyring that is already there.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "T609 measurement: needs Docker and the Ubuntu archive; run by hand"]
async fn t609_kill_during_caddy_install() {
    kill_in_packages("apt-get install caddy", installing_caddy, true).await;
}

/// What happens **today** when a deployment is cancelled during the Packages step: the task
/// runner as shipped (`tasks::deploy::run`, T595 `select!`), cancelled while dpkg is unpacking.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "T609 measurement: needs Docker and the Ubuntu archive; run by hand"]
async fn t609_cancel_today_during_packages() {
    let target = DeployTarget::start(Flavour::Clean).expect("the bare container would not come up");
    let name = target.container_name().to_owned();
    let made = keygen::make("vrcast-studio: T609").expect("no key");
    prewarm(&name, "");

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
        run: vrcast_studio_lib::server::deploy::RunMark::fresh(),
        proofs: Proofs {
            key_works: &key_proof,
            password_refused: &password_proof,
        },
    };
    let steps = steps_for_a_container();
    let task = TaskContext::detached(Arc::new(Db::open_in_memory().unwrap()));
    let token = task.cancel_token();
    let finished = Arc::new(AtomicBool::new(false));

    let started = Instant::now();
    let finished_run = finished.clone();
    let name_run = name.clone();
    let run = async {
        let r =
            vrcast_studio_lib::tasks::deploy::run(&ctx, &steps, &task, &mut |_| {}, &no_second_try)
                .await;
        // The very moment the runner says it is over.
        let at = started.elapsed();
        finished_run.store(true, Ordering::SeqCst);
        let snap = inside(&name_run, PS);
        (r, at, snap)
    };
    let name2 = name.clone();
    let finished_trigger = finished.clone();
    let trigger = async move {
        let found = tokio::task::spawn_blocking(move || {
            wait_until(
                &name2,
                unpacking,
                Duration::from_secs(900),
                Some(&finished_trigger),
            )
        })
        .await
        .unwrap();
        match found {
            Some((_, ps)) => {
                println!(
                    "=== dpkg --unpack seen at {:.1}s; cancelling. Running:\n{ps}",
                    started.elapsed().as_secs_f64()
                );
                token.cancel();
            }
            None => println!("=== the unpack phase was never seen; not cancelling"),
        }
    };
    let ((outcome, at, snap), ()) = tokio::join!(run, trigger);
    println!(
        "RUNNER RETURNED at {:.1}s: {}",
        at.as_secs_f64(),
        match &outcome {
            Ok(_) => String::from("Ok"),
            Err(e) => format!("Err({e:?})"),
        }
    );
    println!("--- apt/dpkg on the server at that moment:\n{snap}");

    // As production does right after the runner returns.
    conn.close().await;
    std::thread::sleep(Duration::from_secs(1));
    println!(
        "--- 1 s after the connection was closed:\n{}",
        inside(&name, PS)
    );

    // Follow it to the end: does apt finish on its own, get killed, or hang?
    let t0 = Instant::now();
    loop {
        let ps = inside(&name, PS);
        if !apt_or_dpkg(&ps) || t0.elapsed() > Duration::from_secs(900) {
            println!(
                "--- apt/dpkg gone (or gave up) {:.1}s after close; last:\n{ps}",
                t0.elapsed().as_secs_f64()
            );
            break;
        }
        std::thread::sleep(Duration::from_secs(1));
    }
    println!("{}", inside(&name, DPKG_STATE));
    println!(
        "--- dpkg.log tail:\n{}",
        inside(&name, "tail -n 5 /var/log/dpkg.log")
    );

    repeat(&target, &made).await;
    println!("--- after the repeat:\n{}", inside(&name, DPKG_STATE));
    println!("{}", inside(&name, SERVING));
}

/// A server that is already deployed and serving; one apt step (fail2ban, the same mechanism
/// as UnattendedUpgrades) is killed in dpkg's configure phase. Does the serving survive, and
/// does a repeat finish it?
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "T609 measurement: needs Docker and the Ubuntu archive; run by hand"]
async fn t609_kill_fail2ban_on_a_serving_server() {
    let target = DeployTarget::start(Flavour::Clean).expect("the bare container would not come up");
    let name = target.container_name().to_owned();
    let made = keygen::make("vrcast-studio: T609").expect("no key");

    prewarm(&name, "fail2ban unattended-upgrades");
    repeat(&target, &made).await;
    println!("--- serving before:\n{}", inside(&name, SERVING));

    // Take fail2ban (and what only it needed) away, the way a drifted server might be.
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
    let Some((after, ps)) = wait_until(&name, configuring, Duration::from_secs(600), None) else {
        println!("RESULT: the configure phase was never seen");
        return;
    };
    println!(
        "=== configure seen {:.1}s after the start; running:\n{ps}",
        after.as_secs_f64()
    );
    println!("{}", inside(&name, STOP));
    after_kill_report(&name);
    println!(
        "--- serving right after the kill:\n{}",
        inside(&name, SERVING)
    );

    repeat(&target, &made).await;
    println!("--- after the repeat:\n{}", inside(&name, DPKG_STATE));
    println!("{}", inside(&name, SERVING));
    println!(
        "fail2ban: {}",
        inside(
            &name,
            "systemctl is-active fail2ban; fail2ban-client status sshd 2>&1 | head -n 2"
        )
    );
}
