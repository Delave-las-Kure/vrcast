//! T601 — `server_rollback` is refused on a server that is not ours to change, before anything
//! is written, and still works on one that is.
//!
//! **The hole.** `server_rollback` opened its session as `Intent::Read`, and the gate lets a
//! read through on anything that answers. What followed was not a read: `upgrade::roll_back`
//! copies `/etc/vrcast/backup/latest/*` — `state.json`, the Caddyfile and the rest — over the
//! live files and reloads the services. So an older application pointed at a server a newer
//! one had upgraded past what it knows (FR-130), or at a machine whose marker it cannot read
//! (FR-132), put back a configuration it had no business touching. `contracts/ipc-commands.md`
//! says a changing command refuses such a server "having done nothing".
//!
//! **Why each refused case lays a backup down first, and it is not decoration.** With nothing
//! under `/etc/vrcast/backup/latest`, `roll_back` stops at "there is nothing to roll back to"
//! — a failure that writes nothing, and the byte comparison below would pass *with the gate
//! torn out*. That is the lesson of `deploy_foreign.rs`: an accident standing where a rule
//! should be. So the backup is there, it differs from the live files, and the only thing
//! left between it and the live configuration is the gate. The error code is checked to be
//! exactly the refusal, not "some error", for the same reason.
//!
//! **Why the plain test container and a key profile, not the foreign deployment fixture.**
//! `server_rollback` asks for the profile's public key right after the gate
//! (`public_key_for`), and a password profile fails there with `InvalidInput` — another
//! accident that would have made the old code look refused. The key profile goes all the way
//! to the copying when the gate lets it, which is what makes these tests red on the old code.
//! Foreignness is produced two ways on the same container: an unreadable `state.json`
//! (`Foreign { StateFileUnreadable }`, the brief's "Foreign, включая StateFileUnreadable"),
//! and a machine stripped of every mark of ours with its web server still answering
//! (`Foreign { WebServerRunning }` — somebody else's machine as the detector really meets one).
//!
//! **The positive control** is the same command on the same container with the shipped
//! `state.json` (version `APP_EXPECTS`, Managed/Ok): the backup's Caddyfile has to arrive —
//! proving the new gate did not close the legitimate rollback along with the illegitimate
//! ones. It needs no deployment: the rollback copies files and its reloads are `|| true`.

use vrcast_studio_lib::commands::deploy::api as deploy;
use vrcast_studio_lib::commands::error::{DetailCode, ErrorCode};
use vrcast_studio_lib::commands::servers::{api as servers, ServerInput};
use vrcast_studio_lib::commands::AppState;
use vrcast_studio_lib::domain::server_profile::AuthKind;
use vrcast_studio_lib::domain::server_state::APP_EXPECTS;
use vrcast_studio_lib::store::db::Db;
use vrcast_studio_lib::store::secrets::InMemorySecretStore;

use super::fixture::{key_path, TestServer, KEY_PASSPHRASE};
use super::library_ops::confirm_fingerprint;

const VIDEO_DIR: &str = "/var/lib/vrcast/videos";

/// What would be different if a rollback had run: every file under the places it writes to,
/// by content. `/etc` is where every file `upgrade::OWNED` names lives; the rest is the same
/// breadth `deploy_foreign.rs` compares, so a restore that wandered is caught too.
const DESCRIBE_YOURSELF: &str = r#"
set -u
for d in /etc /root /srv /usr/local /var/lib /var/www /opt; do
  [ -d "$d" ] || continue
  find "$d" -xdev -type f -print0 2>/dev/null
done | sort -z | xargs -0 -r sha256sum 2>/dev/null | sort
"#;

fn app_state() -> AppState {
    AppState::with_db(
        std::sync::Arc::new(Db::open_in_memory().unwrap()),
        std::sync::Arc::new(InMemorySecretStore::new()),
    )
    .expect("the application state would not assemble")
}

pub(super) async fn setup() -> (TestServer, AppState, String) {
    let server = TestServer::start().expect("the container would not come up");
    let state = app_state();
    let input = ServerInput {
        name: String::from("Container"),
        host: server.host().to_owned(),
        port: server.port,
        user: String::from("root"),
        auth_kind: AuthKind::Key,
        key_path: Some(key_path().to_string_lossy().into_owned()),
        domain: String::from("stream.example.com"),
        video_dir: Some(String::from(VIDEO_DIR)),
        cdn_base: None,
        ipv6_mode: None,
    };
    let id =
        servers::server_add(&state, input, KEY_PASSPHRASE).expect("the profile was not created");
    confirm_fingerprint(&state, &id, &server).await;
    (server, state, id)
}

/// A state file at the given version, in the shape the container ships with.
fn state_json(version: u32) -> String {
    format!(
        r#"{{
  "vrcast_server_version": {version},
  "deployed_at": "2026-08-27T00:00:00Z",
  "deployed_by_app": "0.1.0",
  "steps_applied": ["user-dirs", "configs", "services"],
  "video_dir": "/var/lib/vrcast/videos",
  "domain": "vrcast-server"
}}"#
    )
}

/// Lay a backup down the way `upgrade::back_up` does — a stamped directory and `latest`
/// pointing at it — holding a state file at version 1 and a Caddyfile that differs from the
/// live one. Both differ from what is live in every refused case, so a rollback that ran
/// would show in the comparison.
fn plant_backup(server: &TestServer) {
    let state = state_json(1);
    server
        .exec_inside(&format!(
            "set -e
dir=/etc/vrcast/backup/20260101T000000Z
mkdir -p \"$dir\"
cp -a /etc/caddy/Caddyfile \"$dir/Caddyfile\"
printf '\\n# T601: the backup copy, not the live one\\n' >> \"$dir/Caddyfile\"
cat > \"$dir/state.json\" <<'JSON'
{state}
JSON
ln -sfn \"$dir\" /etc/vrcast/backup/latest"
        ))
        .expect("the backup was not laid down");
}

fn snapshot(server: &TestServer) -> String {
    let said = server
        .exec_inside(DESCRIBE_YOURSELF)
        .expect("the machine would not describe itself");
    // Not `/etc/vrcast/state.json`: the stranger's machine below has none, and that absence
    // is what makes it a stranger's. The backup's copy of it is always there.
    assert!(
        said.contains("/etc/caddy/Caddyfile")
            && said.contains("/etc/vrcast/backup/20260101T000000Z/state.json")
            && said.contains("/etc/vrcast/backup/20260101T000000Z/Caddyfile"),
        "the description of the machine is missing the very files a rollback would change — \
         a comparison of two descriptions that leave them out passes for ever:\n{said}"
    );
    said
}

fn assert_same(before: &str, after: &str, what: &str) {
    if before == after {
        return;
    }
    let was: std::collections::BTreeSet<&str> = before.lines().collect();
    let now: std::collections::BTreeSet<&str> = after.lines().collect();
    let mut said = format!("{what}: the server was changed by a refused rollback:\n");
    for line in now.difference(&was) {
        said.push_str(&format!("  appeared: {line}\n"));
    }
    for line in was.difference(&now) {
        said.push_str(&format!("  gone:     {line}\n"));
    }
    panic!("{said}");
}

#[tokio::test]
async fn a_rollback_is_refused_on_a_server_whose_marker_cannot_be_read_and_changes_nothing() {
    let (server, state, id) = setup().await;
    // Ours in every other way — the limits file and /var/lib/vrcast are there — but the
    // marker cannot be believed. The detector says Foreign { StateFileUnreadable }, and
    // FR-132 says a machine we do not understand is not changed.
    server
        .exec_inside("printf 'this is not json {\\n' > /etc/vrcast/state.json")
        .expect("the state file was not spoilt");
    plant_backup(&server);

    let before = snapshot(&server);
    let outcome = deploy::server_rollback(&state, &id).await;
    let after = snapshot(&server);

    match outcome {
        Err(e)
            if e.code == ErrorCode::ServerForeign
                && e.says(DetailCode::ServerForeignStateUnreadable) => {}
        Err(e) => panic!(
            "refused as {:?} rather than ServerForeign/ServerForeignStateUnreadable — refused \
             by accident is not refused on purpose: {e:?}",
            e.code
        ),
        Ok(()) => panic!("a rollback went through on a server whose marker cannot be read"),
    }
    assert_same(&before, &after, "Foreign (StateFileUnreadable)");
}

#[tokio::test]
async fn a_rollback_is_refused_on_somebody_elses_serving_machine_and_changes_nothing() {
    let (server, state, id) = setup().await;
    // A stranger's machine as the detector sees one: no state file, none of the marks only
    // this application leaves (the limits file, `/var/lib/vrcast`), and a web server
    // answering on 80 — Foreign { WebServerRunning }. The backup under /etc/vrcast is what a
    // previous tenant, or anybody, might have left; a rollback must not reach for it.
    server
        .exec_inside(
            "set -e
rm -f /etc/vrcast/state.json /etc/caddy/vrcast-limits.conf
rm -r /var/lib/vrcast",
        )
        .expect("the machine was not made a stranger's");
    plant_backup(&server);

    let before = snapshot(&server);
    let outcome = deploy::server_rollback(&state, &id).await;
    let after = snapshot(&server);

    match outcome {
        Err(e)
            if e.code == ErrorCode::ServerForeign
                && e.says(DetailCode::ServerForeignWebServerRunning) => {}
        Err(e) => panic!(
            "refused as {:?} rather than ServerForeign/ServerForeignWebServerRunning — refused \
             by accident is not refused on purpose: {e:?}",
            e.code
        ),
        Ok(()) => panic!("a rollback went through on somebody else's machine"),
    }
    assert_same(&before, &after, "Foreign (WebServerRunning)");
}

#[tokio::test]
async fn a_rollback_is_refused_on_a_newer_server_side_and_changes_nothing() {
    let (server, state, id) = setup().await;
    // Upgraded by a newer application to a layout this one does not know (FR-130).
    let newer = state_json(APP_EXPECTS + 1);
    server
        .exec_inside(&format!(
            "cat > /etc/vrcast/state.json <<'JSON'\n{newer}\nJSON"
        ))
        .expect("the newer state file was not written");
    plant_backup(&server);

    let before = snapshot(&server);
    let outcome = deploy::server_rollback(&state, &id).await;
    let after = snapshot(&server);

    match outcome {
        Err(e) if e.code == ErrorCode::ServerTooNew => {}
        Err(e) => panic!(
            "refused as {:?} rather than ServerTooNew — refused by accident is not refused on \
             purpose: {e:?}",
            e.code
        ),
        Ok(()) => panic!("a rollback went through on a server side newer than this application"),
    }
    assert_same(&before, &after, "TooNew");
}

#[tokio::test]
async fn a_rollback_still_goes_through_on_our_own_current_server() {
    // The positive control: the gate that now refuses the two cases above must not have
    // closed the one a rollback exists for (FR-133).
    let (server, state, id) = setup().await;
    plant_backup(&server);
    let expected = server
        .exec_inside("cat /etc/vrcast/backup/latest/Caddyfile")
        .expect("the backup Caddyfile would not read");
    let live_before = server
        .exec_inside("cat /etc/caddy/Caddyfile")
        .expect("the live Caddyfile would not read");
    assert_ne!(
        expected, live_before,
        "the backup is the same as the live file, so a restore would prove nothing"
    );

    deploy::server_rollback(&state, &id)
        .await
        .expect("a rollback on our own current server was refused");

    let live_after = server
        .exec_inside("cat /etc/caddy/Caddyfile")
        .expect("the live Caddyfile would not read");
    assert_eq!(
        live_after, expected,
        "the rollback reported success and the Caddyfile was not put back"
    );
}
