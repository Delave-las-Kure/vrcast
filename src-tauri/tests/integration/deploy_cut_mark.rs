//! T617 — a first deployment broken off right after `Packages` on a bare machine: the stop is
//! confirmed **through the production gate** (`gate::open_to_stop`, by
//! `commands::deploy::stop_through_gate`, exactly what `start`'s `stop_again` calls) and a
//! repeat goes to the end.
//!
//! QA-19 №2: `Packages` installs Caddy — `/etc/caddy/Caddyfile`, a server on 80/443 — and the
//! marks of ours (`/var/lib/vrcast`, `vrcast-limits.conf`) came later. A run broken off in
//! between left a machine `detect` read as `Foreign`; `open_to_stop` refused; the stop was
//! retried for ever and the task held the server for ever. `deploy_cut_stop.rs` did not see
//! it: its stop goes round the gate. Here nothing does. The mark is now the run's first change,
//! so the machine is `Unfinished` — ours, open to the stop and to being finished.
//!
//! **The break** is the run's own SSH session killed on the server (`pkill` of its `sshd`
//! processes) the moment `Packages` settles — between two steps, so nothing of the run is
//! running and the stop is confirmed at the first fresh attempt; what is checked is that the
//! gate lets that attempt in. The old code never lets it in: the run is given
//! [`STOP_WITHIN`] to answer, and a refusal by the gate is failed on its own.
//!
//! Needs Docker and the Ubuntu archive (fetched ahead with retries; "Failed to fetch … 503"
//! is the network, not the code).

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures::future::BoxFuture;
use vrcast_studio_lib::commands::deploy::{api as deploy_api, stop_through_gate};
use vrcast_studio_lib::commands::servers::{api as servers, ServerInput};
use vrcast_studio_lib::commands::AppState;
use vrcast_studio_lib::domain::deploy_steps::{PlannedStep, Status, StepId};
use vrcast_studio_lib::domain::dns_verdict::{Ipv6Choice, ServerAddresses};
use vrcast_studio_lib::domain::marked::Stopped;
use vrcast_studio_lib::domain::server_profile::AuthKind;
use vrcast_studio_lib::domain::server_state::Kind;
use vrcast_studio_lib::server::deploy::{machine, Context, Proofs, RunMark};
use vrcast_studio_lib::server::marked::Patience;
use vrcast_studio_lib::ssh::keygen;
use vrcast_studio_lib::store::db::Db;
use vrcast_studio_lib::store::secrets::InMemorySecretStore;
use vrcast_studio_lib::tasks::engine::TaskContext;

use super::deploy_cancel_measure::{
    inside, looks_like_network, prewarm, repeat_once, steps_for_a_container, DOMAIN,
};
use super::deploy_clean::{by_password, key_works, password_refused, VIDEO_DIR};
use super::deploy_fixture::{DeployTarget, Flavour, ROOT_PASSWORD};

/// How long the run has, from the break, to answer. The stop is confirmed at the first fresh
/// attempt (2 s after the first one through the dead connection); a gate that refuses retries
/// for ever.
const STOP_WITHIN: Duration = Duration::from_secs(120);

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_run_broken_off_after_packages_on_a_bare_machine_is_stopped_through_the_gate() {
    let target = DeployTarget::start(Flavour::Clean).expect("the bare container would not come up");
    let name = target.container_name().to_owned();
    prewarm(&name, "fail2ban unattended-upgrades");

    let state = AppState::with_db(
        Arc::new(Db::open_in_memory().unwrap()),
        Arc::new(InMemorySecretStore::new()),
    )
    .expect("no state");
    let id = servers::server_add(
        &state,
        ServerInput {
            name: String::from("T617 container"),
            host: String::from("127.0.0.1"),
            port: target.port,
            user: String::from("root"),
            auth_kind: AuthKind::Password,
            key_path: None,
            domain: String::from(DOMAIN),
            video_dir: None,
            cdn_base: None,
            ipv6_mode: None,
        },
        ROOT_PASSWORD,
    )
    .expect("the profile was not created");
    let seen = vrcast_studio_lib::commands::api::server_probe_fingerprint("127.0.0.1", target.port)
        .await
        .expect("no fingerprint");
    servers::server_fingerprint_confirm(&state, &id, &seen).expect("not confirmed");
    let profile = vrcast_studio_lib::store::profiles::get(&state.db, &id)
        .unwrap()
        .expect("the profile vanished");
    assert_eq!(
        deploy_api::server_detect(&state, &id).await.unwrap().kind,
        Kind::Clean
    );

    let made = keygen::make("vrcast-studio: T617").expect("no key");
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
    let steps = steps_for_a_container();
    let task = TaskContext::detached(Arc::new(Db::open_in_memory().unwrap()));

    // The production stop, gate and all.
    let attempts = AtomicUsize::new(0);
    let answers: Mutex<Vec<String>> = Mutex::default();
    let stop_again = |mark: String, patience: Patience| -> BoxFuture<'_, Result<Stopped, String>> {
        let state = &state;
        let profile = &profile;
        let attempts = &attempts;
        let answers = &answers;
        Box::pin(async move {
            attempts.fetch_add(1, Ordering::SeqCst);
            let r = stop_through_gate(state, profile, None, &mark, patience).await;
            answers.lock().unwrap().push(format!("{r:?}"));
            r
        })
    };

    let broke_at: Mutex<Option<Instant>> = Mutex::default();
    let after_break: Mutex<Option<String>> = Mutex::default();
    let mut report = |settled: &[PlannedStep]| {
        let Some(last) = settled.last() else { return };
        println!("{:?}: {:?}", last.id, last.status);
        if last.id == StepId::Packages && broke_at.lock().unwrap().is_none() {
            assert_eq!(last.status, Status::Applied, "Packages did not install");
            // What the machine is at the moment of the break, asked without a connection.
            *after_break.lock().unwrap() = Some(inside(
                &name,
                "printf 'mark=%s caddyfile=%s serving=%s\\n' \
                   \"$(test -d /var/lib/vrcast && echo yes || echo no)\" \
                   \"$(test -f /etc/caddy/Caddyfile && echo yes || echo no)\" \
                   \"$(ss -ltnH | awk '$4 ~ /:(80|443)$/' | wc -l)\"",
            ));
            let killed = inside(&name, "pkill -f 'sshd: root' ; echo pkill=$?");
            println!("the run's SSH session killed: {}", killed.trim());
            *broke_at.lock().unwrap() = Some(Instant::now());
        }
    };

    let outcome = tokio::time::timeout(
        STOP_WITHIN + Duration::from_secs(900),
        vrcast_studio_lib::tasks::deploy::run(&ctx, &steps, &task, &mut report, &stop_again),
    )
    .await;
    let answered = Instant::now();
    conn.close().await;
    let broke = broke_at
        .lock()
        .unwrap()
        .expect("the run never reached the end of Packages");
    let seen_then = after_break.lock().unwrap().clone().unwrap_or_default();
    println!(
        "at the break: {}; answered {:.1}s after it; stop attempts: {}; {:?}",
        seen_then.trim(),
        (answered - broke).as_secs_f64(),
        attempts.load(Ordering::SeqCst),
        answers.lock().unwrap()
    );

    assert!(
        seen_then.contains("mark=yes") && seen_then.contains("caddyfile=yes"),
        "the break did not come where it was meant to — Caddy's file there, and our mark: \
         {seen_then}"
    );
    let outcome = outcome.expect("the run never answered: its stop was never confirmed");
    assert!(
        answered - broke < STOP_WITHIN,
        "the stop took {:?} to be confirmed",
        answered - broke
    );
    let error = outcome.expect_err("a run broken off after Packages came back Ok");
    println!("the run's answer: {error:?}");
    assert!(
        attempts.load(Ordering::SeqCst) >= 1,
        "the stop through the dead connection was taken as confirmed"
    );
    assert!(
        answers
            .lock()
            .unwrap()
            .iter()
            .all(|a| !a.contains("gate would not open")),
        "the production gate refused the stop of our own run: {:?}",
        answers.lock().unwrap()
    );
    // And the machine is ours and unfinished — not somebody else's — to anyone who looks.
    assert_eq!(
        deploy_api::server_detect(&state, &id).await.unwrap().kind,
        Kind::Unfinished
    );

    // At once, the repeat, to the end.
    for attempt in 1..=3 {
        let (done, text) = repeat_once(&target, &made).await;
        println!("[repeat {attempt}] {text}");
        if done {
            return;
        }
        assert!(
            looks_like_network(&text) && attempt < 3,
            "the repeat did not reach the end, and not for a network reason: {text}"
        );
    }
}
