//! T300 / SC-016 — a server that is not ours comes out of the run byte for byte the same.
//!
//! ⚠ **The half of T300 nobody had built, found by the audit of 2026-09-04 (T486).** The task
//! was marked done and promised three things. Two existed: a foreign machine is recognised
//! (`detect_live.rs`) and the fixture is genuinely foreign (`deploy_stand.rs`). The third —
//! "the server after the run is byte for byte the same, and that is the content of SC-016" —
//! existed nowhere, and it is the one the principle actually rests on.
//!
//! **Why the refusal is not the thing to check.** `tests/unit/gate.rs` checks the decision on
//! a state assembled by hand, and `tests/unit/gate.rs` is where that belongs. But a decision
//! is a promise about behaviour, and the behaviour is what somebody's machine experiences.
//! Between the two lie the connection, the probe that runs before the refusal, the wording of
//! the refusal itself and every path that reaches a server — and a mistake anywhere along
//! there produces a refusal that reads perfectly and a machine that was written to anyway.
//! SC-016 says "not once does it change a server recognised as foreign". Only a comparison
//! can say that.
//!
//! **The two halves are not one check said twice, and the measurement below says why.** The
//! refusals catch the gate being wrong; the comparison catches everything that reaches the
//! machine *outside* a refusal — the probe that runs before the decision, a step that wrote
//! before it stopped, a path added later that never asked. Neither covers the other's ground.
//!
//! **What is compared, and what is not.** Every file under the directories a deployment owns
//! or would create — `/etc`, `/root`, `/srv`, `/usr/local`, `/var/lib`, `/var/www`, `/opt` —
//! by content, not by date; plus the enabled units, the installed packages and what is
//! listening. Deploying touches all four. Not compared: `/proc`, `/sys`, `/dev`, `/run`,
//! `/tmp` and the logs, which move on their own and which a login writes to whatever it does
//! afterwards — a login is not a change to the server, and treating it as one would make this
//! check red for ever, which teaches people not to read red.

use vrcast_studio_lib::commands::deploy::api as deploy;
use vrcast_studio_lib::commands::error::ErrorCode;
use vrcast_studio_lib::commands::library::api as library;
use vrcast_studio_lib::commands::limits::{api as limits, LimitRequest};
use vrcast_studio_lib::commands::servers::{api as servers, ServerInput};
use vrcast_studio_lib::commands::AppState;
use vrcast_studio_lib::domain::dns_verdict::Ipv6Choice;
use vrcast_studio_lib::domain::server_profile::AuthKind;
use vrcast_studio_lib::store::db::Db;
use vrcast_studio_lib::store::secrets::InMemorySecretStore;

use super::deploy_fixture::{DeployTarget, Flavour, ROOT_PASSWORD};

/// Everything about the machine that a deployment would change, in one string.
///
/// **Content, not timestamps.** A file rewritten with the same bytes is not a change worth
/// failing over, and a file whose mtime moved because something read it is not a change at
/// all. What matters is whether the machine would behave differently afterwards.
const DESCRIBE_YOURSELF: &str = r#"
set -u
for d in /etc /root /srv /usr/local /var/lib /var/www /opt; do
  [ -d "$d" ] || continue
  find "$d" -xdev -type f -print0 2>/dev/null
done | sort -z | xargs -0 -r sha256sum 2>/dev/null | sort
echo '--- enabled units ---'
systemctl list-unit-files --state=enabled --no-legend --no-pager 2>/dev/null | sort
echo '--- packages ---'
dpkg-query -W -f '${Package} ${Version}\n' 2>/dev/null | sort
echo '--- listening ---'
ss -ltnH 2>/dev/null | awk '{print $4}' | sort
"#;

fn snapshot(target: &DeployTarget) -> String {
    target
        .exec_inside(DESCRIBE_YOURSELF)
        .expect("the machine would not describe itself")
}

fn app_state() -> AppState {
    AppState::with_db(
        std::sync::Arc::new(Db::open_in_memory().unwrap()),
        std::sync::Arc::new(InMemorySecretStore::new()),
    )
    .expect("the application state would not assemble")
}

/// A profile pointing at somebody else's machine, the way a person would type one.
///
/// By password, because that is all this machine offers: nobody put our key on it, which is
/// the whole point of it being somebody else's.
async fn profile_for(state: &AppState, target: &DeployTarget) -> String {
    let (host, port) = target.address();
    let id = servers::server_add(
        state,
        ServerInput {
            name: String::from("Somebody else's"),
            host: host.clone(),
            port,
            user: String::from("root"),
            auth_kind: AuthKind::Password,
            key_path: None,
            domain: String::from("stream.example.com"),
            video_dir: Some(String::from("/var/lib/vrcast/videos")),
            cdn_base: None,
            ipv6_mode: None,
        },
        ROOT_PASSWORD,
    )
    .expect("the profile was not created");

    let fingerprint = vrcast_studio_lib::commands::api::server_probe_fingerprint(&host, port)
        .await
        .expect("the fingerprint was not obtained");
    servers::server_fingerprint_confirm(state, &id, &fingerprint)
        .expect("the fingerprint was not confirmed");
    id
}

/// Let every task that started finish, so the machine is looked at when it is done being
/// changed rather than while it still is.
async fn wait_for_any_tasks(state: &AppState) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(900);
    loop {
        let tasks = vrcast_studio_lib::commands::api::tasks_list(state).unwrap_or_default();
        let busy: Vec<&str> = tasks
            .iter()
            .filter(|t| !t.state.is_final())
            .map(|t| t.id.as_str())
            .collect();
        if busy.is_empty() {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "a task started against somebody else's server and has not finished: {busy:?}"
        );
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
}

#[tokio::test]
async fn a_server_that_is_not_ours_is_the_same_afterwards_to_the_byte() {
    let target =
        DeployTarget::start(Flavour::Foreign).expect("the foreign container would not come up");
    let state = app_state();
    let id = profile_for(&state, &target).await;

    let before = snapshot(&target);
    assert!(
        before.contains("--- packages ---") && before.lines().count() > 100,
        "the description of the machine came back too thin to be worth comparing; a check \
         that compares two empty strings passes for ever:\n{before}"
    );

    // Everything a person can press that reaches a server to change it. Each is expected to
    // be refused — and the refusal is not what is being checked here, the machine is.
    let mut refusals: Vec<(&str, Result<String, String>)> = Vec::new();

    refusals.push((
        "deploy_plan",
        deploy::deploy_plan(&state, &id, Ipv6Choice::Keep)
            .await
            .map(|p| format!("{p:?}"))
            .map_err(|e| format!("{:?}", e.code)),
    ));
    refusals.push((
        "deploy_run",
        deploy::deploy_run(&state, &id, Ipv6Choice::Keep, true)
            .await
            .map_err(|e| format!("{:?}", e.code)),
    ));
    refusals.push((
        "media_create",
        library::media_create(&state, &id, "Чужой сервер", None)
            .await
            .map_err(|e| format!("{:?}", e.code)),
    ));
    refusals.push((
        "media_delete",
        library::media_delete(&state, &id, "anything", true)
            .await
            .map_err(|e| format!("{:?}", e.code)),
    ));
    refusals.push((
        "limit_set",
        limits::limit_set(
            &state,
            LimitRequest {
                server_id: id.clone(),
                ip: String::from("203.0.113.7"),
                slug: String::from("anything"),
                cap_bps: 2_000_000,
            },
            true,
        )
        .await
        .map(|()| String::new())
        .map_err(|e| format!("{:?}", e.code)),
    ));

    // Reading is allowed on somebody else's machine and must stay allowed — refusing it is
    // how a person is left unable to find out what they are looking at. It is here because it
    // reaches the server too, and reaching must not change anything either.
    let _ = library::library_list(&state, &id, true).await;

    // ⚠ **Wait for whatever got through, before looking at the machine.** `deploy_run` hands
    // the work to a task and returns; refused, there is no task and this returns at once. But
    // the day the refusal stops working is the day this check matters, and on that day the
    // snapshot would be taken while the deployment was still running — the machine would come
    // back unchanged and the comparison below would pass over a server being rebuilt.
    wait_for_any_tasks(&state).await;

    // ⚠ **Refused by the right rule, not merely refused** — and this is not pedantry, it is
    // what the whole check turns on. Measured 2026-09-04 by opening the gate to a foreign
    // server on purpose: not one of these five reached the machine even then. `deploy_run`
    // stopped at `DomainNotPointed`, `media_create` at `FileMissingOnServer`, `limit_set` at
    // `NoLadderForMedia`, the other two at `InvalidInput`. Five accidents standing where a
    // rule should be — none of them about whose machine it is, every one of them removable by
    // a change that has nothing to do with ownership. A check that accepted any failure would
    // have gone on passing with the gate torn out, and would have been reporting on those
    // accidents while appearing to report on principle I.
    let foreign = format!("{:?}", ErrorCode::ServerForeign);
    let wrong: Vec<String> = refusals
        .iter()
        .filter_map(|(what, outcome)| match outcome {
            Ok(_) => Some(format!("{what} went through on somebody else's server")),
            Err(code) if *code != foreign => Some(format!(
                "{what} was refused as {code} rather than {foreign} — refused by accident is \
                 not refused on purpose, and the accident can be fixed"
            )),
            Err(_) => None,
        })
        .collect();
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));

    // ---- and now the only thing that settles SC-016 --------------------------------------
    let after = snapshot(&target);
    if before != after {
        let mut said = String::from("somebody else's server was changed:\n");
        let was: std::collections::BTreeSet<&str> = before.lines().collect();
        let now: std::collections::BTreeSet<&str> = after.lines().collect();
        for line in now.difference(&was) {
            said.push_str(&format!("  appeared: {line}\n"));
        }
        for line in was.difference(&now) {
            said.push_str(&format!("  gone:     {line}\n"));
        }
        panic!("{said}");
    }
}
