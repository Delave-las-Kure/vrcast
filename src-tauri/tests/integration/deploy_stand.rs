//! T244, T245, T247, T249 — a check of the stand itself, before anything is checked on it.
//!
//! Everything the deployment checks will say rests on these two containers being what they
//! claim: that the bare one is bare, that the other one really is somebody else's, that
//! systemd is up so services can be started at all, and that a reset really returns to bare.
//! An unchecked measuring instrument is not a measuring instrument — the same lesson as
//! T149a, and it cost a task there.
//!
//! Nothing about the application is checked here, deliberately: a failure has to say which
//! of the two broke.

use super::deploy_fixture::{only_the_stand, DeployTarget, Flavour, ROOT_PASSWORD};

#[test]
fn a_bare_server_carries_nothing_of_ours() {
    let target = DeployTarget::start(Flavour::Clean).expect("the bare container would not come up");

    // Asked for through the guarded accessor rather than read off the field: that is the way
    // a deployment test will obtain it, and it is what puts the guard on the path.
    let (host, port) = target.address();
    assert_eq!(host, "127.0.0.1");
    assert!(port > 0, "no port was published");

    // Every one of these is something a deployment step has to create. A step whose work is
    // already done proves nothing about the step, and — worse — the recognition rule would
    // read the container as deployed or as somebody else's and never as clean.
    for path in [
        "/etc/vrcast/state.json",
        "/etc/vrcast",
        "/etc/caddy",
        "/var/lib/vrcast",
    ] {
        assert!(
            !target
                .has(path)
                .expect("could not look inside the container"),
            "{path} is there, so the container is not bare"
        );
    }

    let caddy = target
        .exec_inside("command -v caddy || echo none")
        .expect("could not look for caddy");
    assert_eq!(caddy.trim(), "none", "Caddy is already installed");

    // Nothing listening on 80 — the second branch of the recognition rule keys off a running
    // web server, and a container with one would be recognised as foreign.
    let listening = target
        .exec_inside("ss -ltn 2>/dev/null | grep -c ':80 ' || true")
        .expect("could not look at the listening ports");
    assert_eq!(
        listening.trim(),
        "0",
        "something is already listening on port 80"
    );

    // No key, on purpose: a freshly bought server has only what the provider put there, and
    // putting the key in is the application's own step — the one the whole ordering rule
    // exists for (ssh-key before ssh-hardening, R-12). With a key already there that step
    // would have nothing to do and its check nothing to see.
    assert!(
        !target
            .has("/root/.ssh/authorized_keys")
            .expect("could not look for the key"),
        "the key is already in place, so the ssh-key step has nothing to do"
    );
}

#[test]
fn systemd_is_up_and_a_unit_can_be_started() {
    // **This is what T246 rests on.** If systemd is not running here then services, the
    // network filter and the hardening are not checkable by machine at all, and the whole
    // of phase 7 falls to the one test VPS — which is one machine, by hand, and not in
    // continuous integration.
    let target = DeployTarget::start(Flavour::Clean).expect("the bare container would not come up");

    let pid1 = target
        .exec_inside("cat /proc/1/comm")
        .expect("could not ask who PID 1 is");
    assert_eq!(pid1.trim(), "systemd", "PID 1 is not systemd");

    let state = target
        .exec_inside("systemctl is-system-running || true")
        .expect("could not ask systemd how it is");
    assert!(
        matches!(state.trim(), "running" | "degraded"),
        "systemd did not finish coming up: {state:?}"
    );

    // Not "the unit is enabled" but "it stops and starts": enabling writes a symlink, and a
    // symlink is not a service.
    target
        .exec_inside("systemctl stop ssh && systemctl start ssh")
        .expect("a unit would not stop and start again");
    let active = target
        .exec_inside("systemctl is-active ssh")
        .expect("could not ask about the unit");
    assert_eq!(active.trim(), "active", "the unit did not come back up");
}

#[test]
fn somebody_elses_server_serves_somebody_elses_site() {
    let target =
        DeployTarget::start(Flavour::Foreign).expect("the foreign container would not come up");

    // The finding behind the refusal has to be real (FR-132). A forged answer from the
    // detector would check the wording of the refusal and tell us nothing about whether the
    // application sees a foreign server at all.
    let active = target
        .exec_inside("systemctl is-active nginx")
        .expect("could not ask about nginx");
    assert_eq!(active.trim(), "active", "nginx is not running");

    let answer = target
        .exec_inside("curl -s -o /dev/null -w '%{http_code}' http://127.0.0.1/")
        .expect("could not ask the foreign server");
    assert_eq!(answer.trim(), "200", "the foreign server does not answer");

    // No state file, and there must never be one: with it the server would be recognised as
    // ours and this image would stop being what it is for.
    assert!(
        !target
            .has("/etc/vrcast/state.json")
            .expect("could not look for the state file"),
        "the foreign server has a state file, so it is not foreign any more"
    );
    // And no Caddyfile either — that is the *other* branch of the rule, covered by the
    // ordinary fixture. Here the finding must be the running web server alone, or this
    // container stops checking the branch it was made for.
    assert!(
        !target
            .has("/etc/caddy/Caddyfile")
            .expect("could not look for the Caddyfile"),
        "the foreign server has a Caddyfile, so it no longer checks the branch it was made for"
    );
}

#[test]
fn a_reset_really_returns_to_bare() {
    // Safety on a repeat (FR-124) is checked by **interrupting** a deployment, and the only
    // thing there is to interrupt is one that began on a bare machine. Without a reset that
    // can be believed, the second case of every such check would start from wherever the
    // first one left off.
    let mut target =
        DeployTarget::start(Flavour::Clean).expect("the bare container would not come up");

    target
        .exec_inside("mkdir -p /etc/vrcast /etc/caddy && echo '{}' > /etc/vrcast/state.json")
        .expect("could not dirty the container");
    assert!(
        target
            .has("/etc/vrcast/state.json")
            .expect("could not look inside"),
        "the container would not be dirtied, so the reset proves nothing"
    );

    let before = target.port;
    target.reset().expect("the reset failed");

    assert!(
        !target
            .has("/etc/vrcast/state.json")
            .expect("could not look inside after the reset"),
        "the reset left what was written before it"
    );
    // The published port changes with the container, and anything holding the old one has to
    // read it again. Said here rather than in a comment alone, because a stale port fails as
    // "connection refused" — which reads as a broken server, not as a stale number.
    assert_ne!(
        before, target.port,
        "the port did not change, so the container was not actually replaced"
    );
}

#[test]
fn the_password_the_fixture_knows_is_the_one_the_images_set() {
    // A comment in three files saying "these must be changed together" is not a mechanism.
    // A fresh server is reached by password — that is the only way in before the `ssh-key`
    // step runs — so a password that has drifted apart from the fixture's idea of it fails
    // every deployment check at the door, with "authentication failed" and nothing to say
    // which of the two is wrong.
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let expected = format!("root:{ROOT_PASSWORD}");
    for image in ["docker-clean", "docker-foreign"] {
        let dockerfile = dir.join(image).join("Dockerfile");
        let text = std::fs::read_to_string(&dockerfile)
            .unwrap_or_else(|e| panic!("could not read {}: {e}", dockerfile.display()));
        assert!(
            text.contains(&expected),
            "{image}/Dockerfile does not set the password the fixture expects ({ROOT_PASSWORD})"
        );
    }
}

#[test]
fn a_bare_server_still_lets_a_password_in() {
    // Not "the password is set" but "sshd will accept one". The hardening step's whole job is
    // to take this away (T274), and a check of that is worth nothing unless it was there to
    // begin with.
    let target = DeployTarget::start(Flavour::Clean).expect("the bare container would not come up");
    let effective = target
        .exec_inside("sshd -T | grep -E '^(passwordauthentication|permitrootlogin)'")
        .expect("could not read the effective sshd configuration");
    assert!(
        effective.contains("passwordauthentication yes"),
        "password logins are already off on a bare server: {effective:?}"
    );
    assert!(
        effective.contains("permitrootlogin yes"),
        "root cannot log in on a bare server: {effective:?}"
    );
}

#[test]
#[should_panic(expected = "the throwaway stand only")]
fn the_guard_refuses_an_address_that_is_not_the_stand() {
    // T249. Deployment is the one thing the application does that can take a working server
    // down and lock its owner out, so the address is checked rather than trusted. A test
    // that the guard actually bites: without one it is a comment with an `assert!` next to
    // it.
    only_the_stand("203.0.113.10");
}

#[test]
fn the_guard_lets_the_stand_through() {
    // The other half, and not a formality: a guard that refuses everything would pass the
    // check above and quietly make every deployment test impossible to write.
    only_the_stand("127.0.0.1");
    only_the_stand("localhost");
    // And the other shape the stand is reached by: a container this module named itself, on a
    // network it made. Left out, the guard would refuse every deployment test the moment they
    // run inside a container — which is where they run in continuous integration.
    only_the_stand("vrcast-deploy-target-931-2");
}
