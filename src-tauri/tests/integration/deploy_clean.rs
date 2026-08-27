//! T298 — the deployment, on a bare machine.
//!
//! The mechanism is checked without a server (`tests/unit/deploy_engine.rs`); the steps
//! themselves cannot be. What each of them does is run somebody else's program and read what
//! it says about the result, and neither half can be stood in for: a fake apt would tell us
//! about our fake.
//!
//! **What this container cannot answer, it says so about** (T246, measured): swap, the kernel
//! settings and the disk's readahead are not establishable inside any container on any host,
//! and the serving over a domain has neither a domain nor a certificate here. Those come back
//! as "cannot be established here" — not as done — and that distinction is checked below,
//! because folded into "done" a container run would report a fully deployed server that has
//! none of them.
//!
//! The rest is real: packages installed from the distribution's archives, a system user,
//! directories with their owners, the reference configuration validated by Caddy itself, the
//! service enabled and running, our key in place and proved by a fresh login, password logins
//! turned off and proved refused, the firewall with both families, and the state file last.

use std::time::Duration;

use futures::future::BoxFuture;
use vrcast_studio_lib::domain::deploy_steps::{SkipReason, Status, StepId};
use vrcast_studio_lib::domain::dns_verdict::{Ipv6Choice, ServerAddresses};
use vrcast_studio_lib::server::deploy::{self, machine, Context, Proofs};
use vrcast_studio_lib::ssh::{fingerprint, Connection, Credentials, ServerAddress};

use super::deploy_fixture::{DeployTarget, Flavour, ROOT_PASSWORD};
use super::test_key::{key_path, public_key_path, PASSPHRASE};

const VIDEO_DIR: &str = "/var/lib/vrcast/videos";

async fn address(target: &DeployTarget) -> ServerAddress {
    let (host, port) = target.address();
    ServerAddress::new(host, port)
}

/// A connection made with the password, the way a person first reaches a bought server.
async fn by_password(target: &DeployTarget) -> Connection {
    let a = address(target).await;
    let fp = fingerprint::probe(&a)
        .await
        .expect("the fingerprint was not obtained");
    Connection::connect(
        a,
        "root",
        Credentials::Password(ROOT_PASSWORD.to_owned()),
        &fp,
    )
    .await
    .expect("the password would not get us in to a freshly made server")
}

/// Whether a fresh connection with the key works. **A new one every time** — the connection we
/// already hold would go on working whatever we did to the settings, which is what makes it
/// the wrong witness.
async fn key_works(target: &DeployTarget) -> bool {
    let a = address(target).await;
    let Ok(fp) = fingerprint::probe(&a).await else {
        return false;
    };
    Connection::connect(
        a,
        "root",
        Credentials::Key {
            path: key_path(),
            passphrase: Some(PASSPHRASE.to_owned()),
        },
        &fp,
    )
    .await
    .is_ok()
}

/// Whether a password is actually refused.
async fn password_refused(target: &DeployTarget) -> bool {
    let a = address(target).await;
    let Ok(fp) = fingerprint::probe(&a).await else {
        return false;
    };
    Connection::connect(
        a,
        "root",
        Credentials::Password(ROOT_PASSWORD.to_owned()),
        &fp,
    )
    .await
    .is_err()
}

#[tokio::test]
async fn a_bare_machine_is_deployed_and_a_repeat_does_nothing() {
    let mut target =
        DeployTarget::start(Flavour::Clean).expect("the bare container would not come up");
    // The key has to exist before it can be put anywhere.
    super::test_key::ensure().expect("the test key was not made");
    let public_key = std::fs::read_to_string(public_key_path()).expect("no public key");

    let conn = by_password(&target).await;
    let machine = machine::look(&conn)
        .await
        .expect("the machine would not describe itself");
    assert!(
        machine.is_container(),
        "the container did not recognise itself as one, so the steps that cannot run here \
         would be attempted and would fail for the wrong reason"
    );

    let key_proof = || -> BoxFuture<'_, bool> { Box::pin(key_works(&target)) };
    let password_proof = || -> BoxFuture<'_, bool> { Box::pin(password_refused(&target)) };
    let ctx = Context {
        conn: &conn,
        // Nothing points at a container, and the domain step is not what this checks. It is
        // left out of the run below rather than pretended at.
        domain: "vrcast-container.invalid",
        video_dir: VIDEO_DIR,
        ipv6: Ipv6Choice::Keep,
        server: ServerAddresses { v4: None, v6: None },
        public_key: public_key.clone(),
        machine,
        proofs: Proofs {
            key_works: &key_proof,
            password_refused: &password_proof,
        },
    };

    // Everything but the two steps that ask the outside world about a domain this machine
    // does not have. Dropping them here is not hiding them: they are checked in
    // `tests/unit/dns_verdict.rs` and on the stand by hand (T332).
    let steps: Vec<_> = deploy::all()
        .into_iter()
        .filter(|s| !matches!(s.id, StepId::DnsCheck | StepId::Verify))
        .collect();

    let never = || false;
    let done = deploy::run(&ctx, &steps, &never, &mut |_| {})
        .await
        .expect("the deployment failed");

    // Nothing was left not applied. A step reported as still to do after a successful run is
    // the mechanism lying about itself.
    for step in &done {
        assert!(
            !matches!(step.status, Status::NotApplied | Status::Failed { .. }),
            "{:?} came back as {:?}",
            step.id,
            step.status
        );
    }

    // The three that cannot be settled here say so — and are **not** called done.
    for id in [StepId::Swap, StepId::Tuning] {
        let step = done.iter().find(|s| s.id == id).expect("step missing");
        assert!(
            matches!(
                step.status,
                Status::Skipped {
                    why: SkipReason::NotPossibleHere { .. }
                }
            ),
            "{id:?} came back as {:?} in a container",
            step.status
        );
    }

    // And the rest really happened, asked of the machine rather than of our own report.
    let seen = target
        .exec_inside(
            "id vrcast >/dev/null 2>&1 && echo user
test -d /var/lib/vrcast/videos && echo videos
test -f /etc/caddy/Caddyfile && echo caddyfile
test -f /etc/caddy/vrcast-limits.conf && echo limits
[ \"$(systemctl is-active caddy)\" = active ] && echo serving
sshd -T | grep -qx 'passwordauthentication no' && echo password-off
ufw status | head -n 1 | grep -q 'Status: active' && echo firewall
test -f /etc/vrcast/state.json && echo state",
        )
        .expect("the machine would not answer");
    for expected in [
        "user",
        "videos",
        "caddyfile",
        "limits",
        "serving",
        "password-off",
        "firewall",
        "state",
    ] {
        assert!(
            seen.contains(expected),
            "{expected} is missing after a successful deployment: {seen}"
        );
    }

    // **The repeat** (FR-124, SC-015). Nothing was remembered between the two runs: the
    // checks look at the server.
    let again = deploy::run(&ctx, &steps, &never, &mut |_| {})
        .await
        .expect("the repeat failed");
    for step in &again {
        assert!(
            matches!(step.status, Status::Applied | Status::Skipped { .. }),
            "{:?} was done again on a repeat: {:?}",
            step.id,
            step.status
        );
    }

    // The container is put back so a later check in this file starts from bare.
    let _ = target.reset();
    let _ = Duration::from_secs(0);
}
