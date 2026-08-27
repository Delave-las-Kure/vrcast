//! T283–T285 — upgrading a server that is already ours, and putting it back.
//!
//! Three promises are checked here, and each of them is the kind that is believed rather than
//! noticed when it is broken:
//!
//! - **nothing raised is lost** (FR-131, SC-017). A video and a catalogue are put on the
//!   server before the upgrade and are still there, byte for byte, afterwards. An upgrade
//!   that quietly took a catalogue with it would turn a person's whole library into
//!   "unrecognised" — the files would all still be there, and every one of them nameless;
//! - **what is replaced can be put back** (FR-133);
//! - **a file somebody edited is not ours to overwrite**. A person who tuned their own web
//!   server and found the application had silently undone it would be right to stop trusting
//!   it (T285).

use futures::future::BoxFuture;
use vrcast_studio_lib::domain::deploy_steps::StepId;
use vrcast_studio_lib::domain::dns_verdict::{Ipv6Choice, ServerAddresses};
use vrcast_studio_lib::server::deploy::{self, machine, Context, DeployError, Proofs};
use vrcast_studio_lib::server::upgrade;
use vrcast_studio_lib::ssh::Connection;

use super::deploy_clean::{by_password, key_works, password_refused, VIDEO_DIR};
use super::deploy_fixture::{DeployTarget, Flavour};
use vrcast_studio_lib::ssh::keygen;

/// A machine deployed once, ready to be upgraded.
/// A machine reached by password, and a key made for it the way the application makes one
/// (T290a). The private half is handed back so the proof can sign in with it.
async fn deployed(target: &DeployTarget) -> (Connection, keygen::MadeKey) {
    let made = keygen::make("vrcast-studio: the check").expect("no key was made");
    let conn = by_password(target).await;
    (conn, made)
}

/// Everything a container can carry out. The two steps that ask the outside world about
/// a domain this machine does not have are left off rather than pretended at.
fn steps_for_a_container<'a>() -> Vec<deploy::Step<Context<'a>>> {
    deploy::all()
        .into_iter()
        .filter(|s| !matches!(s.id, StepId::DnsCheck | StepId::Verify))
        .collect()
}

/// Only the steps that put the configuration files there.
///
/// Two of the checks below are about **files** — replacing them, putting them back,
/// refusing to touch an edited one — and a whole deployment before each of them costs two
/// minutes apiece to establish something they do not look at. What they do need is the
/// directories and the configuration, so that is what they run.
fn steps_for_files<'a>(with_state: bool) -> Vec<deploy::Step<Context<'a>>> {
    deploy::all()
        .into_iter()
        .filter(|s| {
            // Packages too, and not for their own sake: the directories step hands the
            // log directory to the `caddy` user, who does not exist until the package
            // creates them. Leaving it out failed with an empty message, which is how
            // both of those were found.
            matches!(s.id, StepId::Packages | StepId::UserDirs | StepId::Configs)
                || (with_state && s.id == StepId::State)
        })
        .collect()
}

#[tokio::test]
async fn an_upgrade_keeps_every_video_and_the_catalogue() {
    let target = DeployTarget::start(Flavour::Clean).expect("the bare container would not come up");
    let (conn, made) = deployed(&target).await;
    let machine = machine::look(&conn).await.expect("no machine facts");

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
        machine,
        already_ours: false,
        proofs: Proofs {
            key_works: &key_proof,
            password_refused: &password_proof,
        },
    };
    let steps = steps_for_a_container();
    let never = || false;

    deploy::run(&ctx, &steps, &never, &mut |_| {})
        .await
        .expect("the first deployment failed");

    // A person's work, put there between the deployment and the upgrade.
    target
        .exec_inside(&format!(
            "printf 'a film, more or less' > {VIDEO_DIR}/film.mp4
printf '{{\"generation\":7,\"media\":[{{\"id\":\"m_1\",\"title\":\"Фильм\"}}]}}' > {VIDEO_DIR}/library.json
sha256sum {VIDEO_DIR}/film.mp4 {VIDEO_DIR}/library.json | cut -d' ' -f1"
        ))
        .expect("could not put a film and a catalogue on the server");
    let before = target
        .exec_inside(&format!(
            "sha256sum {VIDEO_DIR}/film.mp4 {VIDEO_DIR}/library.json | cut -d' ' -f1"
        ))
        .expect("could not read the sums");

    // The upgrade: on a server already ours, this time.
    let ctx = Context {
        already_ours: true,
        ..ctx
    };
    let plan = upgrade::plan(&ctx, 1, &steps)
        .await
        .expect("the upgrade plan failed");
    assert!(
        !plan.has_work(),
        "a server just deployed by this very version has work waiting: {:?}",
        plan.steps
            .iter()
            .filter(|s| matches!(
                s.status,
                vrcast_studio_lib::domain::deploy_steps::Status::NotApplied
            ))
            .map(|s| s.id)
            .collect::<Vec<_>>()
    );

    upgrade::run(&ctx, &steps, &never, &mut |_| {})
        .await
        .expect("the upgrade failed");

    let after = target
        .exec_inside(&format!(
            "sha256sum {VIDEO_DIR}/film.mp4 {VIDEO_DIR}/library.json | cut -d' ' -f1"
        ))
        .expect("could not read the sums after");
    assert_eq!(
        before, after,
        "the upgrade changed a video or the catalogue (FR-131, SC-017)"
    );

    // And the backup really holds what it promised.
    let kept = target
        .exec_inside("ls /etc/vrcast/backup/latest/ | sort")
        .expect("no backup was made");
    for name in ["Caddyfile", "state.json", "00-vrcast.conf"] {
        assert!(kept.contains(name), "{name} is not in the backup: {kept}");
    }
    // **And the person's work is NOT in it.** A backup that swept up the catalogue would, on
    // a rollback, undo everything uploaded since — which is the opposite of a rescue.
    assert!(
        !kept.contains("library.json") && !kept.contains("film.mp4"),
        "the backup swept up somebody's library: {kept}"
    );
}

#[tokio::test]
async fn a_rollback_puts_the_replaced_files_back() {
    let target = DeployTarget::start(Flavour::Clean).expect("the bare container would not come up");
    let (conn, made) = deployed(&target).await;
    let machine = machine::look(&conn).await.expect("no machine facts");

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
        machine,
        already_ours: false,
        proofs: Proofs {
            key_works: &key_proof,
            password_refused: &password_proof,
        },
    };
    let steps = steps_for_files(true);
    let never = || false;
    deploy::run(&ctx, &steps, &never, &mut |_| {})
        .await
        .expect("laying the configuration down failed");

    upgrade::back_up(&ctx).await.expect("the backup failed");

    // Something is replaced, the way an upgrade would replace it.
    target
        .exec_inside("printf 'a later version wrote this\\n' > /etc/vrcast/state.json")
        .expect("could not change the state file");

    upgrade::roll_back(&ctx).await.expect("the rollback failed");

    let back = target
        .exec_inside("cat /etc/vrcast/state.json")
        .expect("could not read the state file");
    assert!(
        back.contains("vrcast_server_version"),
        "the rollback did not put the state file back: {back}"
    );
}

#[tokio::test]
async fn a_configuration_edited_by_hand_is_refused_rather_than_overwritten() {
    // **T285.** The application owns this file only in the sense that it created it; once a
    // person has edited it, it is theirs. Noticing and saying so is the promise; quietly
    // putting our version back is the failure — and it is a failure nobody sees until their
    // own tuning stops working.
    let target = DeployTarget::start(Flavour::Clean).expect("the bare container would not come up");
    let (conn, made) = deployed(&target).await;
    let machine = machine::look(&conn).await.expect("no machine facts");

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
        machine,
        already_ours: false,
        proofs: Proofs {
            key_works: &key_proof,
            password_refused: &password_proof,
        },
    };
    let steps = steps_for_files(false);
    let never = || false;
    deploy::run(&ctx, &steps, &never, &mut |_| {})
        .await
        .expect("laying the configuration down failed");

    // The person adds a header of their own.
    target
        .exec_inside(
            "printf '\\n# my own line\\n' >> /etc/caddy/Caddyfile && caddy validate --adapter caddyfile --config /etc/caddy/Caddyfile >/dev/null 2>&1 || true",
        )
        .expect("could not edit the configuration");
    let theirs = target
        .exec_inside("sha256sum /etc/caddy/Caddyfile | cut -d' ' -f1")
        .expect("could not read the sum");

    let ctx = Context {
        already_ours: true,
        ..ctx
    };
    let outcome = deploy::run(&ctx, &steps, &never, &mut |_| {}).await;
    match outcome {
        Err(DeployError::Step { id, detail, .. }) => {
            assert_eq!(id, StepId::Configs);
            assert!(
                detail.contains("edited by hand"),
                "the refusal does not say why: {detail}"
            );
        }
        other => panic!("a hand-edited configuration was accepted: {other:?}"),
    }

    let still = target
        .exec_inside("sha256sum /etc/caddy/Caddyfile | cut -d' ' -f1")
        .expect("could not read the sum after");
    assert_eq!(
        theirs, still,
        "the hand-edited configuration was overwritten anyway"
    );
}
