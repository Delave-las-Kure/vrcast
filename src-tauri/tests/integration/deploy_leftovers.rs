//! T622 — the next run tidies what an interrupted write of **ours** left half-written, by
//! name, and nothing else: an administrator's own `*.vrcast.tmp` in `/etc` survives the run.
//!
//! A bare container, three files planted before a full deployment with the production steps
//! through the production `tasks::deploy::run` (`repeat_once`, the same run the T609/T615
//! tests repeat with): our leftover beside a path the run writes, and two foreign files with
//! the same suffix — one in a directory of its own (QA-19 №7's example), one beside our
//! leftover in the same directory, so the survivor is kept by its name and not by where it is.
//!
//! Needs Docker and the Ubuntu archive (a failure with "Failed to fetch … 503" or "Hash Sum
//! mismatch" is the network, retried up to three times).

use super::deploy_cancel_measure::{inside, looks_like_network, prewarm, repeat_once};
use super::deploy_fixture::{DeployTarget, Flavour};
use vrcast_studio_lib::server::deploy::{TEMP_PLACES, TEMP_SUFFIX};
use vrcast_studio_lib::ssh::keygen;

/// Ours: a path the run writes (the tuning step's sysctl file), with the suffix.
const OURS: &str = "/etc/sysctl.d/99-vrcast-net.conf";
/// Somebody else's, with the same suffix: the example from the audit…
const FOREIGN: &str = "/etc/admin-backups/Caddyfile.vrcast.tmp";
/// …and one in the same directory as ours.
const FOREIGN_BESIDE: &str = "/etc/sysctl.d/99-admin.conf.vrcast.tmp";

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_run_removes_its_own_leftover_and_leaves_a_foreign_vrcast_tmp_alone() {
    assert!(TEMP_PLACES.contains(&OURS));
    let target = DeployTarget::start(Flavour::Clean).expect("the bare container would not come up");
    let name = target.container_name().to_owned();
    prewarm(&name, "fail2ban unattended-upgrades");

    inside(
        &name,
        &format!(
            "mkdir -p /etc/admin-backups /etc/sysctl.d && echo admin > {FOREIGN} \
             && echo admin > {FOREIGN_BESIDE} && echo half > {OURS}{TEMP_SUFFIX}"
        ),
    );

    let made = keygen::make("vrcast-studio: T622").expect("no key");
    for attempt in 1..=3 {
        let (done, text) = repeat_once(&target, &made).await;
        println!("[run {attempt}] {text}");
        if done {
            break;
        }
        assert!(
            looks_like_network(&text) && attempt < 3,
            "the run did not reach the end, and not for a network reason: {text}"
        );
    }

    let ours = inside(&name, &format!("ls {OURS}{TEMP_SUFFIX} 2>&1"));
    assert!(
        ours.contains("No such file"),
        "our half-written leftover was not tidied: {ours}"
    );
    for foreign in [FOREIGN, FOREIGN_BESIDE] {
        let left = inside(&name, &format!("cat {foreign} 2>&1"));
        assert_eq!(
            left.trim(),
            "admin",
            "a foreign {foreign} did not survive the run: {left}"
        );
    }
}
