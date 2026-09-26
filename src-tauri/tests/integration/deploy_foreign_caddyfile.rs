//! T611 — a first deployment over a Caddyfile that is somebody else's.
//!
//! The check that refuses to write over a hand-edited Caddyfile (T285) used to run only on a
//! server already ours. A first deployment wrote over whatever was there. The owner's decision
//! (2026-09-23): the plan says the file is somebody else's, the screen asks "replace it (a copy
//! is kept)", and the `configs` step refuses exactly as it does on ours unless the person
//! ticked it.
//!
//! How such a machine comes about: `detect` calls a Caddyfile with none of our marks
//! `Foreign` and nothing is deployed over it. A first deployment meets one only on an
//! `Unfinished` server — a run stopped after the packages and directories steps, and the
//! file edited (or written by somebody) before the next one. That is what is built here:
//! the packages and directories steps first, then the file.
//!
//! The path is the one a real run takes: `upgrade::run` makes the copy first, then the steps.

use futures::future::BoxFuture;
use vrcast_studio_lib::domain::deploy_steps::StepId;
use vrcast_studio_lib::domain::dns_verdict::{Ipv6Choice, ServerAddresses};
use vrcast_studio_lib::server::deploy::{self, configs, machine, Context, DeployError, Proofs};
use vrcast_studio_lib::server::upgrade;
use vrcast_studio_lib::ssh::keygen;

use super::deploy_clean::{by_password, key_works, password_refused, VIDEO_DIR};
use super::deploy_fixture::{DeployTarget, Flavour};

const DOMAIN: &str = "vrcast-container.invalid";

/// Somebody's own serving: valid for Caddy, recognisable in a copy, and none of ours.
const THEIRS: &str = ":8080 {\n\trespond \"somebody else's serving\"\n}\n";

fn steps<'a>(ids: &[StepId]) -> Vec<deploy::Step<Context<'a>>> {
    deploy::all()
        .into_iter()
        .filter(|s| ids.contains(&s.id))
        .collect()
}

fn sum(target: &DeployTarget) -> String {
    target
        .exec_inside("sha256sum /etc/caddy/Caddyfile | cut -d' ' -f1")
        .expect("could not read the Caddyfile's sum")
}

#[tokio::test]
async fn t611_a_first_deployment_replaces_somebody_elses_caddyfile_only_when_agreed() {
    let target = DeployTarget::start(Flavour::Clean).expect("the bare container would not come up");
    let made = keygen::make("vrcast-studio: the check").expect("no key was made");
    let conn = by_password(&target).await;
    let machine = machine::look(&conn).await.expect("no machine facts");

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
        machine,
        already_ours: false,
        replace_caddyfile: false,
        run: deploy::RunMark::fresh(),
        proofs: Proofs {
            key_works: &key_proof,
            password_refused: &password_proof,
        },
    };
    let never = || false;

    // The run that stopped before its configuration step.
    deploy::run(
        &ctx,
        &steps(&[StepId::Packages, StepId::UserDirs]),
        &never,
        &mut |_| {},
    )
    .await
    .expect("the packages and directories steps failed");

    // What the caddy package itself laid down is nobody's work: no question is asked about a
    // file this very deployment installed.
    assert_eq!(
        configs::existing(&ctx).await.expect("could not look"),
        configs::Existing::PackageDefault,
        "the caddy package's own untouched Caddyfile was not recognised as such"
    );
    assert!(
        !configs::needs_consent(&ctx).await.expect("could not ask"),
        "the plan would ask about the Caddyfile the caddy package installed"
    );

    // Somebody writes their own.
    target
        .exec_inside(&format!(
            "printf '%s' '{}' > /etc/caddy/Caddyfile",
            THEIRS.replace('\'', "'\\''")
        ))
        .expect("could not write somebody else's Caddyfile");
    let theirs = sum(&target);
    assert!(
        configs::needs_consent(&ctx).await.expect("could not ask"),
        "the plan does not say the Caddyfile is somebody else's (DeployPreview.foreign_caddyfile)"
    );

    // **Without the tick**: refused at the configuration step, in the words used on a server
    // already ours, and the file is not touched.
    let outcome = upgrade::run(&ctx, &steps(&[StepId::Configs]), &never, &mut |_| {}).await;
    match outcome {
        Err(DeployError::Step { id, detail, .. }) => {
            assert_eq!(id, StepId::Configs);
            assert!(
                detail.contains("edited by hand"),
                "the refusal is not the one a hand-edited file gets on ours: {detail}"
            );
        }
        other => panic!("somebody else's Caddyfile was replaced without agreement: {other:?}"),
    }
    assert_eq!(
        theirs,
        sum(&target),
        "the Caddyfile was changed although the step refused"
    );

    // **With the tick**: replaced, and the copy the run took first holds theirs.
    let agreed = Context {
        replace_caddyfile: true,
        run: deploy::RunMark::fresh(),
        ..ctx
    };
    upgrade::run(&agreed, &steps(&[StepId::Configs]), &never, &mut |_| {})
        .await
        .expect("the configuration step refused a Caddyfile the person agreed to replace");

    let now = target
        .exec_inside("cat /etc/caddy/Caddyfile")
        .expect("could not read the Caddyfile");
    assert!(
        vrcast_studio_lib::server::deploy::references::is_ours(&now, DOMAIN),
        "the Caddyfile after an agreed replacement is not ours:\n{now}"
    );
    let kept = target
        .exec_inside("cat /etc/vrcast/backup/latest/Caddyfile")
        .expect("the replaced Caddyfile is in no copy");
    assert_eq!(
        kept, THEIRS,
        "the copy holds something other than the file that was replaced"
    );

    // And "put it back as it was" does put theirs back.
    upgrade::roll_back(&agreed)
        .await
        .expect("the rollback failed");
    assert_eq!(
        theirs,
        sum(&target),
        "the rollback did not put somebody else's Caddyfile back"
    );
}
