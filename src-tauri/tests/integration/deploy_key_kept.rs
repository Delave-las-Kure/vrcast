//! T616 — the key a deployment makes for a server reached by password is kept (the system's
//! store, the profile switched to it) before `SshHardening` turns password logins off; a run
//! that goes no further than that — the application closed, the task dropped — leaves a
//! profile that a new start of the application signs in with.
//!
//! QA-19 №1: the key used to be kept only after the whole run returned. Closed after
//! `SshHardening`, the application took the only copy of the private key with it: the profile
//! still said "password", and the server no longer took one.
//!
//! **The run is stopped right after `SshHardening` settles** — the `cancelled` answer of the
//! production `server::deploy::run` turns true, so nothing after it is sent — and everything
//! of that run is then dropped: its connection, its context with the made key in memory, its
//! application state. What is left is what a closed application leaves: the database and the
//! secret store. A new application state over those two signs in through the production gate
//! (`server_detect`, `gate::open`, `connect_raw` with the profile's own credentials).
//!
//! Needs Docker and the Ubuntu archive (the Packages step installs for real).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use futures::future::BoxFuture;
use vrcast_studio_lib::commands::deploy::{api as deploy_api, key_keeper};
use vrcast_studio_lib::commands::servers::{api as servers, ServerInput};
use vrcast_studio_lib::commands::AppState;
use vrcast_studio_lib::domain::deploy_steps::{Status, StepId};
use vrcast_studio_lib::domain::dns_verdict::{Ipv6Choice, ServerAddresses};
use vrcast_studio_lib::domain::server_profile::AuthKind;
use vrcast_studio_lib::domain::server_state::Kind;
use vrcast_studio_lib::server::deploy::{self, machine, Context, Proofs, RunMark};
use vrcast_studio_lib::ssh::keygen;
use vrcast_studio_lib::store::db::Db;
use vrcast_studio_lib::store::secrets::{InMemorySecretStore, SecretStore};

use super::deploy_clean::{by_password, key_works, password_refused, VIDEO_DIR};
use super::deploy_fixture::{DeployTarget, Flavour, ROOT_PASSWORD};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_run_closed_after_the_hardening_step_leaves_a_profile_that_signs_in_with_the_key() {
    let target = DeployTarget::start(Flavour::Clean).expect("the bare container would not come up");
    let db = Arc::new(Db::open_in_memory().unwrap());
    let secrets: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::new());

    let id = {
        let state = AppState::with_db(db.clone(), secrets.clone()).expect("no state");
        let id = servers::server_add(
            &state,
            ServerInput {
                name: String::from("T616 container"),
                host: String::from("127.0.0.1"),
                port: target.port,
                user: String::from("root"),
                auth_kind: AuthKind::Password,
                key_path: None,
                domain: String::from("vrcast-container.invalid"),
                video_dir: None,
                cdn_base: None,
                ipv6_mode: None,
            },
            ROOT_PASSWORD,
        )
        .expect("the profile was not created");
        let seen =
            vrcast_studio_lib::commands::api::server_probe_fingerprint("127.0.0.1", target.port)
                .await
                .expect("no fingerprint");
        servers::server_fingerprint_confirm(&state, &id, &seen).expect("not confirmed");

        // The run, as `commands::deploy::start` builds it for a password profile.
        let profile = vrcast_studio_lib::store::profiles::get(&db, &id)
            .unwrap()
            .expect("the profile vanished");
        let made = keygen::make("vrcast-studio: T616").expect("no key");
        let conn = by_password(&target).await;
        let facts = machine::look(&conn).await.expect("no machine facts");
        let key_proof =
            || -> BoxFuture<'_, bool> { Box::pin(key_works(&target, &made.private_openssh)) };
        let password_proof = || -> BoxFuture<'_, bool> { Box::pin(password_refused(&target)) };
        let ctx = Context {
            conn: &conn,
            domain: "vrcast-container.invalid",
            video_dir: VIDEO_DIR,
            ipv6: Ipv6Choice::Keep,
            server: ServerAddresses { v4: None, v6: None },
            public_key: made.public_openssh.clone(),
            machine: facts,
            already_ours: false,
            replace_caddyfile: false,
            run: RunMark::fresh().keeping_key(key_keeper(
                &state,
                &profile,
                made.private_openssh.clone(),
            )),
            proofs: Proofs {
                key_works: &key_proof,
                password_refused: &password_proof,
            },
        };
        let steps: Vec<_> = deploy::all()
            .into_iter()
            .filter(|s| !matches!(s.id, StepId::DnsCheck | StepId::Verify))
            .collect();

        // "Closed" the moment `SshHardening` has settled: nothing after it is sent.
        let hardened = AtomicBool::new(false);
        let closed = || hardened.load(Ordering::SeqCst);
        let mut watch = |step: &vrcast_studio_lib::domain::deploy_steps::PlannedStep| {
            println!("{:?}: {:?}", step.id, step.status);
            if step.id == StepId::SshHardening {
                assert_eq!(
                    step.status,
                    Status::Applied,
                    "the hardening step did not take"
                );
                hardened.store(true, Ordering::SeqCst);
            }
        };
        let outcome = deploy::run(&ctx, &steps, &closed, &mut watch).await;
        assert!(
            matches!(outcome, Err(deploy::DeployError::Cancelled)),
            "the run was not stopped after the hardening step: {outcome:?}"
        );
        conn.close().await;
        // Everything of the run goes here — the made key in memory with it.
        id
    };

    // Password logins really are off: without the kept key there would be no way in.
    assert!(
        password_refused(&target).await,
        "the server still takes a password"
    );

    // A new start of the application: the same database, the same secret store, nothing else.
    let state = AppState::with_db(db.clone(), secrets.clone()).expect("no state");
    let profile = vrcast_studio_lib::store::profiles::get(&db, &id)
        .unwrap()
        .expect("the profile vanished");
    assert_eq!(
        profile.auth_kind,
        AuthKind::ManagedKey,
        "the profile still signs in with a password the server no longer takes"
    );
    assert!(profile.key_path.is_none());
    let found = deploy_api::server_detect(&state, &id)
        .await
        .expect("a new start of the application could not sign in to the server");
    assert_eq!(
        found.kind,
        Kind::Unfinished,
        "the half-deployed server was not recognised as ours and unfinished: {found:?}"
    );
}
