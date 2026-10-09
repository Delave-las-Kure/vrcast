//! T703 — an upgrade from version 1 of the server side, live, on a container with systemd.
//!
//! Version 2 exists because of the owner's decision Д1: nothing under `/videos/` may be
//! declared immutable, since «Заменить» puts a new film at the same addresses. Three things
//! have to hold for that to reach a server already deployed, and none of them is visible from
//! a file:
//!
//! - the application has to **recognise** a version-1 server as one it may upgrade — the
//!   card offers «Обновить» and the upgrade's own gate has to let it through;
//! - the upgrade has to **recognise version 1's Caddyfile as ours** and replace it, rather
//!   than refuse it as edited by hand;
//! - and the new rules have to be **in force**, not merely on disk: the `services` step
//!   finds Caddy serving our domain and does nothing, so without a reload the running server
//!   would go on answering `immutable` until somebody restarted it.
//!
//! And then the point of it all: after a piece is replaced, a player that asks again with the
//! old piece's ETag gets the new piece, not a 304.

use futures::future::BoxFuture;
use vrcast_studio_lib::domain::deploy_steps::{Status, StepId};
use vrcast_studio_lib::domain::dns_verdict::{Ipv6Choice, ServerAddresses};
use vrcast_studio_lib::domain::server_state::{self, Compat, Kind, APP_EXPECTS};
use vrcast_studio_lib::server::deploy::{
    self, machine, packages, references, updates, Context, Proofs,
};
use vrcast_studio_lib::server::gate::{allowed, Intent};
use vrcast_studio_lib::server::{detect, upgrade};
use vrcast_studio_lib::ssh::keygen;

use super::deploy_clean::{by_password, key_works, password_refused, VIDEO_DIR};
use super::deploy_fixture::{DeployTarget, Flavour};

/// A `.localhost` name: Caddy gives it a certificate from its own internal authority on the
/// spot, so the serving can be asked over HTTPS inside the container. A name that wanted a
/// public certificate would get none here, and nothing could be asked at all.
const DOMAIN: &str = "vrcast-upgrade.localhost";

/// The Caddy version 1's server is given here: older than the pin, so the upgrade has to
/// bring it up (T713).
const OLD_CADDY: &str = "2.11.4";

/// What the serving answers for one address, headers and body, asked from inside.
fn ask(target: &DeployTarget, path: &str, extra: &str) -> String {
    target
        .exec_inside(&format!(
            "curl -sk -D - --max-time 10 --resolve {DOMAIN}:443:127.0.0.1 {extra} \
             https://{DOMAIN}/videos/{path}"
        ))
        .unwrap_or_else(|e| panic!("the serving would not answer for {path}: {e}"))
}

fn header<'a>(answer: &'a str, name: &str) -> Option<&'a str> {
    answer.lines().find_map(|l| {
        let (k, v) = l.split_once(':')?;
        k.trim().eq_ignore_ascii_case(name).then(|| v.trim())
    })
}

fn state_of(target: &DeployTarget) -> server_state::ServerState {
    let said = target
        .exec_inside(&detect::command(VIDEO_DIR))
        .expect("the machine would not answer");
    server_state::judge(&detect::read(&said))
}

/// Ask until the serving answers at all — a reload hands the certificate out again, and the
/// first question after it can come before the answer is ready.
fn wait_for_serving(target: &DeployTarget, path: &str) -> String {
    for _ in 0..30 {
        let said = ask(target, path, "");
        if said.starts_with("HTTP/") && !said.contains(" 502") {
            return said;
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
    panic!("the serving never answered for {path}");
}

#[tokio::test]
async fn an_upgrade_from_version_1_puts_the_new_caching_rules_in_force() {
    // There has to be a version 1 to upgrade from.
    const _: () = assert!(APP_EXPECTS >= 2);
    let target = DeployTarget::start(Flavour::Clean).expect("the bare container would not come up");
    let made = keygen::make("vrcast-studio: the check").expect("no key was made");
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
        run: deploy::RunMark::fresh(),
        proofs: Proofs {
            key_works: &key_proof,
            password_refused: &password_proof,
        },
    };
    // What the serving needs and nothing more: the directories, the configuration, the
    // service and the state file. The hardening steps are checked elsewhere and change
    // nothing about what is served.
    //
    // ⚠ **Version 1's packages are put on by hand, the way version 1 put them** (T713): the
    // distribution's packages, Caddy older than the pin from Caddy's own release package
    // (the repository answers 402, so it cannot be had from there), and Caddy's apt list and
    // key left behind — the list that now fails every `apt-get update` on the owner's server.
    target
        .exec_inside(&format!(
            "set -e
export DEBIAN_FRONTEND=noninteractive
apt-get -o Acquire::Retries=5 update -qq
apt-get -o Acquire::Retries=5 install -y -qq {names} {updates} >/dev/null
curl -fsSL -o /tmp/caddy.deb \
  https://github.com/caddyserver/caddy/releases/download/v{OLD_CADDY}/caddy_{OLD_CADDY}_linux_amd64.deb
dpkg -i /tmp/caddy.deb >/dev/null
echo installed",
            names = packages::FROM_APT.join(" "),
            updates = updates::PACKAGE,
        ))
        .expect("could not put version 1's packages on the stand");
    let pick = |with_packages: bool| -> Vec<_> {
        deploy::all()
            .into_iter()
            .filter(|s| {
                (with_packages && s.id == StepId::Packages)
                    || matches!(
                        s.id,
                        StepId::UserDirs
                            | StepId::Configs
                            | StepId::Services
                            | StepId::UnattendedUpgrades
                            | StepId::State
                    )
            })
            .collect()
    };
    let steps = pick(true);
    let never = || false;
    deploy::run(&ctx, &pick(false), &never, &mut |_| {})
        .await
        .expect("laying the serving down failed");

    // ---- A version-1 server, the way version 1 left one: its Caddyfile, its rules file
    // with the header alone, its state file. Put back by our own means — the point is what
    // the upgrade does with them, not how they got there.
    let v1 = references::caddyfile(1, DOMAIN).expect("version 1's Caddyfile is not kept");
    ctx.put_file("/etc/caddy/Caddyfile", &v1)
        .await
        .expect("could not put version 1's Caddyfile back");
    ctx.put_file(
        "/etc/caddy/vrcast-limits.conf",
        "# The quality-limit rules. This file belongs to VRCast Studio: it is rewritten whole\n\
         # on every change, and anything added here by hand will be lost.\n",
    )
    .await
    .expect("could not put version 1's rules file back");
    target
        .exec_inside(&format!(
            "sed -i 's/\"vrcast_server_version\": *[0-9]*/\"vrcast_server_version\": 1/' /etc/vrcast/state.json \
             && systemctl reload caddy \
             && mkdir -p {VIDEO_DIR}/film \
             && printf '#EXTM3U\\n' > {VIDEO_DIR}/film/master.m3u8 \
             && printf 'the old piece' > {VIDEO_DIR}/film/seg_00001.ts \
             && chown -R caddy:caddy {VIDEO_DIR}/film"
        ))
        .expect("could not make the server a version-1 one");
    // Caddy's apt list and key, by version 1's own two lines (T713). Both addresses still
    // answer; it is the repository behind them that answers 402.
    target
        .exec_inside(&format!(
            "set -e
curl -1sLf https://dl.cloudsmith.io/public/caddy/stable/gpg.key | gpg --batch --yes --dearmor -o {keyring}
curl -1sLf https://dl.cloudsmith.io/public/caddy/stable/debian.deb.txt > {list}
grep -q cloudsmith {list}
echo listed",
            keyring = packages::KEYRING,
            list = packages::OLD_LIST,
        ))
        .expect("could not put version 1's Caddy list on the stand");
    let broken = target
        .exec_inside("apt-get update -qq 2>&1; echo \"exit=$?\"")
        .expect("could not ask apt");
    println!("apt-get update on the version-1 stand:\n{broken}");
    assert!(
        !broken.contains("exit=0") && broken.contains("cloudsmith"),
        "the stand does not have the owner's trouble — apt-get update does not fail on the list:\n{broken}"
    );
    let limits_before = target
        .exec_inside("sha256sum /etc/caddy/vrcast-limits.conf | cut -d' ' -f1")
        .expect("could not read the rules file's sum");

    let before = wait_for_serving(&target, "film/seg_00001.ts");
    assert!(
        header(&before, "cache-control").is_some_and(|v| v.contains("immutable")),
        "the stand is not a version-1 server — it does not answer the way version 1 did:\n{before}"
    );

    // ---- The card: «Обновить», and the door behind it open.
    let behind = state_of(&target);
    assert_eq!(behind.kind, Kind::Managed);
    assert_eq!(behind.server_version, Some(1));
    assert_eq!(behind.compat, Compat::Ok, "version 1 is still worked with");
    assert!(
        behind.upgrade_available,
        "the card would not offer «Обновить»"
    );
    assert!(
        allowed(&behind, Intent::Setup).is_ok(),
        "the upgrade's gate refuses the upgrade the card offers"
    );
    assert!(
        allowed(&behind, Intent::Change).is_ok(),
        "a version-1 server stopped being served from"
    );

    // ---- The plan names the change, and only it.
    let ctx = Context {
        already_ours: true,
        run: deploy::RunMark::fresh(),
        ..ctx
    };
    let plan = upgrade::plan(&ctx, 1, &steps)
        .await
        .expect("the upgrade plan failed");
    let to_do: Vec<StepId> = plan
        .steps
        .iter()
        .filter(|s| matches!(s.status, Status::NotApplied))
        .map(|s| s.id)
        .collect();
    assert_eq!(plan.to, APP_EXPECTS);
    assert_eq!(
        to_do,
        vec![StepId::Packages, StepId::Configs, StepId::State],
        "the upgrade from version 1 is Caddy's list and package, the Caddyfile and the state file"
    );

    upgrade::run(&ctx, &steps, &never, &mut |_| {})
        .await
        .expect("the upgrade failed — on Caddy's dead list, or version 1's Caddyfile refused?");

    // ---- T713: the list and its key are gone, apt works again, Caddy is the pinned one —
    // and the program running is the new one, not the old one still in memory.
    let after_apt = target
        .exec_inside(&format!(
            "ls {list} {keyring} 2>&1 | grep -v 'No such file' ; \
             apt-get update -qq 2>&1; echo \"exit=$?\"; \
             echo \"version=$(dpkg-query -W -f='${{Version}}' caddy)\"; \
             echo \"exe=$(readlink /proc/$(systemctl show -p MainPID --value caddy)/exe)\"",
            list = packages::OLD_LIST,
            keyring = packages::KEYRING,
        ))
        .expect("could not ask apt after the upgrade");
    println!("after the upgrade:\n{after_apt}");
    assert!(
        after_apt.contains("exit=0") && !after_apt.contains("cloudsmith"),
        "apt still fails after the upgrade:\n{after_apt}"
    );
    assert!(
        !after_apt.contains(packages::OLD_LIST) && !after_apt.contains(packages::KEYRING),
        "the old list or its key is still there:\n{after_apt}"
    );
    assert!(
        after_apt.contains(&format!("version={}", packages::CADDY_VERSION)),
        "Caddy is not the pinned version:\n{after_apt}"
    );
    assert!(
        after_apt.contains("exe=/usr/bin/caddy") && !after_apt.contains("(deleted)"),
        "the old Caddy is still the one running:\n{after_apt}"
    );
    // A second run finds nothing to do in the packages: the check sees what the apply did.
    let again = upgrade::plan(&ctx, APP_EXPECTS, &steps)
        .await
        .expect("the plan after the upgrade failed");
    assert!(
        !again.has_work(),
        "the upgrade left work behind: {:?}",
        again.steps
    );

    // ---- On disk, in the state file, and IN FORCE.
    let now = state_of(&target);
    assert_eq!(now.server_version, Some(APP_EXPECTS));
    assert!(!now.upgrade_available, "still offered an upgrade after it");
    let loaded = target
        .exec_inside("curl -s http://127.0.0.1:2019/config/")
        .expect("Caddy's own endpoint would not answer");
    assert!(
        loaded.contains("no-cache") && !loaded.contains("immutable"),
        "the new file is on disk and the running server is still the old one:\n{loaded}"
    );
    let limits_after = target
        .exec_inside("sha256sum /etc/caddy/vrcast-limits.conf | cut -d' ' -f1")
        .expect("could not read the rules file's sum after");
    assert_eq!(
        limits_before, limits_after,
        "the upgrade touched the quality-limit rules (T610)"
    );

    let piece = wait_for_serving(&target, "film/seg_00001.ts");
    let master = ask(&target, "film/master.m3u8", "-o /dev/null");
    for (what, said) in [("a piece", &piece), ("the master", &master)] {
        assert_eq!(
            header(said, "cache-control"),
            Some("no-cache"),
            "{what} is not revalidated:\n{said}"
        );
    }
    let old_etag = header(&piece, "etag")
        .expect("no ETag on a piece: there is nothing to revalidate by")
        .to_owned();

    // Unchanged: a 304, no body — revalidating costs a round trip, not the piece.
    let same = ask(
        &target,
        "film/seg_00001.ts",
        &format!("-H 'If-None-Match: {old_etag}'"),
    );
    assert!(
        same.starts_with("HTTP/2 304") || same.starts_with("HTTP/1.1 304"),
        "{same}"
    );

    // ---- «Заменить»: a new piece at the same address. The old link, asked with the old
    // piece's ETag, brings the new piece.
    target
        .exec_inside(&format!(
            "sleep 1.2 && printf 'the new piece, longer' > {VIDEO_DIR}/film/seg_00001.ts"
        ))
        .expect("could not replace the piece");
    let after = ask(
        &target,
        "film/seg_00001.ts",
        &format!("-H 'If-None-Match: {old_etag}'"),
    );
    assert!(
        after.starts_with("HTTP/2 200") || after.starts_with("HTTP/1.1 200"),
        "a replaced piece was answered as unchanged:\n{after}"
    );
    assert!(after.contains("the new piece, longer"), "{after}");
}
