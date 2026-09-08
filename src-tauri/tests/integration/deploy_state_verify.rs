//! T526 — the state file tells the truth about whether serving was ever confirmed.
//!
//! `Verify` cannot ask anything inside a container: no domain of its own, no certificate,
//! and its check answers `Checked::NotPossibleHere` rather than confirming the serving
//! (`server::deploy::verify`). Until this was fixed the `State` step wrote the same file
//! either way — a promise that "all of this was done here" including the one thing that
//! was never checked. This is the only place that promise can actually be tested: the
//! container is the one case `serving_verified` is meant to catch, and there is no faking
//! a `Connection` to reach `state_file::body` any other way (constitution: logic reachable
//! only through a server counts as unchecked).
//!
//! **Deliberately its own file, not an addition to `deploy_clean.rs`.** That file is
//! T298/T500's and belongs to `vrcast-qa`; this checks one field this task added, and does
//! it by running only the `State` step — the rest of a full deployment has nothing to do
//! with what is being asked here and would only cost the run two more minutes.

use futures::future::BoxFuture;
use vrcast_studio_lib::domain::deploy_steps::StepId;
use vrcast_studio_lib::domain::dns_verdict::{Ipv6Choice, ServerAddresses};
use vrcast_studio_lib::domain::server_state::parse_state_file;
use vrcast_studio_lib::server::deploy::{self, machine, Context, Proofs};
use vrcast_studio_lib::ssh::keygen;

use super::deploy_clean::{by_password, key_works, password_refused, VIDEO_DIR};
use super::deploy_fixture::{DeployTarget, Flavour};

#[tokio::test]
async fn a_container_s_state_file_says_serving_was_not_verified() {
    let target = DeployTarget::start(Flavour::Clean).expect("the bare container would not come up");
    let made = keygen::make("vrcast-studio: the state-file check").expect("no key was made");
    let conn = by_password(&target).await;
    let machine = machine::look(&conn).await.expect("no machine facts");
    assert!(
        machine.is_container(),
        "the container did not recognise itself as one, so this check is not about what \
         it says it is"
    );

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

    // `State` and its one prerequisite: `/etc/vrcast` is made by `UserDirs`, which in turn
    // needs the `caddy` user `Packages` creates (the same ordering `deploy_resume.rs`'s
    // `steps_for_files` uses). `State` itself does not read anything the other steps would
    // have written beyond that directory existing — it stamps the version, the directories
    // from the context, and now this flag — so a whole deployment before it would only cost
    // two minutes establishing what this check does not look at.
    let steps: Vec<_> = deploy::all()
        .into_iter()
        .filter(|s| matches!(s.id, StepId::Packages | StepId::UserDirs | StepId::State))
        .collect();
    let never = || false;
    deploy::run(&ctx, &steps, &never, &mut |_| {})
        .await
        .expect("writing the state file failed");

    let text = target
        .exec_inside("cat /etc/vrcast/state.json")
        .expect("the state file was not written");
    let file = parse_state_file(text.trim()).expect("the state file does not read");
    assert!(
        !file.serving_verified,
        "a container run marked serving as verified, but Verify cannot ask anything here"
    );
}
