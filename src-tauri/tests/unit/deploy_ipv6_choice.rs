//! T525 — the IPv6 choice from a deployment is read back, not re-decided, on upgrade and
//! rollback.
//!
//! `server_upgrade_plan`, `server_upgrade_run` and `server_rollback` used to write
//! `Ipv6Choice::Keep` straight into their `Context` without ever looking at
//! `profile.ipv6_mode` — the choice a person made when the server was first deployed. So a
//! server deployed with IPv6 turned off had its `ipv6` step silently answer `NotNeeded` on
//! every later upgrade and rollback, and nobody confirmed the disabling again: the module
//! that carries out that step says plainly, in its own header, that no path there is a
//! silent default, and this was exactly that — a `Keep` written in three places without
//! reading the profile.
//!
//! `ipv6_choice_of` is the fix: the one place `Ipv6Mode` (the profile's own record of the
//! choice, an `Option`) turns into `Ipv6Choice` (what a deploy step executes, not an
//! option). It is pure and takes no server, so it is checked directly rather than through
//! a live upgrade or rollback — those need a server to reach past this point at all, and
//! what varies here is entirely a property of the profile.

use vrcast_studio_lib::commands::deploy::ipv6_choice_of;
use vrcast_studio_lib::domain::dns_verdict::Ipv6Choice;
use vrcast_studio_lib::domain::server_profile::{AuthKind, Ipv6Mode, ServerProfile};

fn profile_with(ipv6_mode: Option<Ipv6Mode>) -> ServerProfile {
    ServerProfile {
        id: String::from("p1"),
        name: String::from("test"),
        host: String::from("203.0.113.10"),
        port: 22,
        user: String::from("root"),
        auth_kind: AuthKind::Password,
        secret_ref: String::from("ref"),
        key_path: None,
        domain: String::from("stream.example.com"),
        video_dir: String::from("/var/lib/vrcast/videos"),
        cdn_base: None,
        host_fingerprint: None,
        ipv6_mode,
        is_active: false,
    }
}

#[test]
fn a_profile_that_chose_to_disable_ipv6_is_read_as_disable() {
    // The bug this closes: this used to come out `Keep` regardless of what the profile
    // said, in `server_upgrade_plan`, `server_upgrade_run` and `server_rollback` alike.
    let profile = profile_with(Some(Ipv6Mode::Disable));
    assert_eq!(ipv6_choice_of(&profile), Ipv6Choice::Disable);
}

#[test]
fn a_profile_that_chose_to_keep_ipv6_is_read_as_keep() {
    let profile = profile_with(Some(Ipv6Mode::Keep));
    assert_eq!(ipv6_choice_of(&profile), Ipv6Choice::Keep);
}

#[test]
fn a_profile_from_before_the_choice_existed_keeps_ipv6_rather_than_guessing() {
    // `None` means the deployment predates `ipv6_mode` existing at all — there was no
    // choice to record. `Keep` here changes nothing about a server already running with
    // whatever IPv6 state it has, which is the deliberate, documented exception to "no
    // silent default": it is not a guess standing in for an unmade choice, because nobody
    // is being asked to choose right now and nothing is being switched off behind them.
    let profile = profile_with(None);
    assert_eq!(ipv6_choice_of(&profile), Ipv6Choice::Keep);
}
