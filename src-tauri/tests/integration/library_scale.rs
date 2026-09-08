//! T562(б) — library_list(refresh: true) stays fast as the library grows, and getting there
//! concurrently really is faster than one SFTP round trip after another.
//!
//! The audit's own words: "все существующие тесты используют маленькие фикстуры" — every
//! other library test in this suite works with a handful of files, so a return to strictly
//! sequential probing (say, a `.buffered()` call quietly turned back into a plain loop by a
//! future edit) would never show up as a broken assertion; it would only show up as a
//! person's library screen taking longer to open, which nobody writes a bug for right away.
//!
//! **Why this test runs its own latency proxy instead of measuring the raw Docker
//! container.** The whole complaint T562 answers is about *round-trip* cost stacking up —
//! the audit's own text says so ("десятки-сотни последовательных сетевых round-trip'ов
//! подряд"). Loopback Docker on this machine has, for practical purposes, no round-trip time
//! at all, and `viewers_live.rs` already records as fact that this kernel offers no traffic
//! shaping (`no netem, no tbf — measured on 2026-08-26`). Measured without any injected
//! delay, concurrent and serial came out within noise of each other (checked while writing
//! this test: ~0.86s vs ~0.69s over 64 files, i.e. concurrency was NOT reliably faster) —
//! not because the fix does not work, but because with zero latency there is nothing for
//! `.buffered(6)` to overlap. A real VPS the audit describes has real latency, so this test
//! manufactures a small, fixed amount of it with a plain TCP relay that delays every chunk it
//! forwards in both directions, and points the server profile at the relay instead of at the
//! container directly. The relay is protocol-blind (it forwards raw bytes and never parses
//! SSH), so the container's real host key and behaviour are unaffected.
//!
//! Two things are checked:
//!
//! 1. **A generous absolute ceiling** — independent of the comparison below, catches a hang
//!    or a gross slowdown outright.
//! 2. **A relative comparison against a genuinely sequential baseline**, measured in this same
//!    test, through the same relay, right after: the same per-file call
//!    (`probe_moov::params_for`, exactly what `probed()` calls) run one file after another
//!    rather than `.buffered(6)`. No exact percentage is asserted — a loaded CI machine can
//!    slow both sides unevenly — but concurrent must not be slower than serial, which is the
//!    one outcome that would mean the concurrency is not actually taking effect.

use std::sync::Arc;
use std::time::{Duration, Instant};

use super::fixture::{key_path, TestServer, KEY_PASSPHRASE};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use vrcast_studio_lib::commands::library::api as library_api;
use vrcast_studio_lib::commands::servers::{api as servers_api, ServerInput};
use vrcast_studio_lib::commands::AppState;
use vrcast_studio_lib::domain::server_profile::AuthKind;
use vrcast_studio_lib::server::probe_moov;
use vrcast_studio_lib::ssh::{fingerprint, Connection, Credentials, ServerAddress};
use vrcast_studio_lib::store::db::Db;
use vrcast_studio_lib::store::secrets::InMemorySecretStore;

const VIDEO_DIR: &str = "/var/lib/vrcast/videos";

/// More than the "50+" asked for, and not a round number: a count this test invented itself
/// rather than one that happens to match some internal batch size elsewhere.
const FILE_COUNT: usize = 64;

/// Small on purpose. What is under test here is the *number* of round trips and how well
/// they overlap, not bandwidth — a large payload would only add a bandwidth-delay-product
/// cost equally to both the concurrent and the serial measurement and dilute the very
/// difference the test exists to show.
const FILE_BYTES: usize = 4096;

/// One-way delay the relay adds to every chunk it forwards, in each direction — so a single
/// request-then-response round trip through it costs about twice this. Modest and real-VPS
/// shaped (well under, say, a trans-oceanic link), chosen only to be comfortably larger than
/// this machine's own near-zero loopback latency so the two measurements can actually differ.
const RELAY_ONE_WAY_DELAY: Duration = Duration::from_millis(15);

/// A ceiling with a great deal of room, in the spirit of `responsiveness.rs`'s own reasoning
/// ("the difference between tens and hundreds of milliseconds matters more here than the
/// exact figure"): what this test must catch is a regression to strictly serial round trips
/// or a hang, not shave milliseconds off a healthy run.
const GENEROUS_CEILING: Duration = Duration::from_secs(30);

fn app_state() -> AppState {
    AppState::with_db(
        Arc::new(Db::open_in_memory().unwrap()),
        Arc::new(InMemorySecretStore::new()),
    )
    .expect("the application state would not assemble")
}

fn profile_via_relay(relay_port: u16) -> ServerInput {
    ServerInput {
        name: String::from("Container"),
        host: String::from("127.0.0.1"),
        port: relay_port,
        user: String::from("root"),
        auth_kind: AuthKind::Key,
        key_path: Some(key_path().to_string_lossy().into_owned()),
        domain: String::from("stream.example.com"),
        video_dir: Some(String::from(VIDEO_DIR)),
        cdn_base: None,
        ipv6_mode: None,
    }
}

fn file_names() -> Vec<String> {
    (0..FILE_COUNT)
        .map(|i| format!("Episode_{i:03}.mp4"))
        .collect()
}

/// Lay out `FILE_COUNT` files with no valid MP4 header (random bytes): `probed()` will read a
/// head's worth of each over SFTP, fail to parse it, and fall back to blank particulars —
/// exercising exactly the network round trips this test measures, without needing a real
/// encode per file.
fn prepare(server: &TestServer, names: &[String]) {
    for name in names {
        server
            .exec_inside(&format!(
                "head -c {FILE_BYTES} /dev/urandom > '{VIDEO_DIR}/{name}'"
            ))
            .unwrap_or_else(|e| panic!("could not create {name}: {e}"));
    }

    let files_json = names
        .iter()
        .map(|n| format!("\"{n}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let manifest = format!(
        r#"{{
      "generation": 1,
      "media": [
        {{ "id": "m_season", "title": "A Season", "slug": "a-season",
          "files": [{files_json}],
          "ladders": [],
          "created_at": "2026-08-01T10:00:00Z" }}
      ]
    }}"#
    );
    server
        .exec_inside(&format!(
            "cat > '{VIDEO_DIR}/library.json' <<'EOF'\n{manifest}\nEOF"
        ))
        .expect("could not write the catalogue");
}

/// A protocol-blind TCP relay that adds `RELAY_ONE_WAY_DELAY` to every chunk forwarded in
/// each direction, standing in for a real network's propagation delay. It never looks at the
/// bytes — SSH negotiates through it exactly as it would through a slow link.
///
/// Returns the local port to connect to and a handle to stop the relay by dropping/aborting.
async fn start_latency_relay(upstream_port: u16) -> (u16, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("the relay could not bind a local port");
    let local_port = listener
        .local_addr()
        .expect("the relay's local address could not be read")
        .port();

    let handle = tokio::spawn(async move {
        loop {
            let (inbound, _) = match listener.accept().await {
                Ok(pair) => pair,
                Err(_) => break,
            };
            tokio::spawn(async move {
                let outbound = match TcpStream::connect(("127.0.0.1", upstream_port)).await {
                    Ok(s) => s,
                    Err(_) => return,
                };
                let (mut ri, mut wi) = inbound.into_split();
                let (mut ro, mut wo) = outbound.into_split();

                let client_to_server = async move {
                    let mut buf = [0u8; 16 * 1024];
                    loop {
                        let n = match ri.read(&mut buf).await {
                            Ok(0) | Err(_) => break,
                            Ok(n) => n,
                        };
                        tokio::time::sleep(RELAY_ONE_WAY_DELAY).await;
                        if wo.write_all(&buf[..n]).await.is_err() {
                            break;
                        }
                    }
                    let _ = wo.shutdown().await;
                };
                let server_to_client = async move {
                    let mut buf = [0u8; 16 * 1024];
                    loop {
                        let n = match ro.read(&mut buf).await {
                            Ok(0) | Err(_) => break,
                            Ok(n) => n,
                        };
                        tokio::time::sleep(RELAY_ONE_WAY_DELAY).await;
                        if wi.write_all(&buf[..n]).await.is_err() {
                            break;
                        }
                    }
                    let _ = wi.shutdown().await;
                };
                tokio::join!(client_to_server, server_to_client);
            });
        }
    });

    (local_port, handle)
}

/// Confirm the fingerprint the relay's address presents — the same SSH server sits behind
/// it, so the same host key comes through; only the route to it is different.
async fn confirm_fingerprint_via_relay(state: &AppState, server_id: &str, relay_port: u16) {
    let fingerprint =
        vrcast_studio_lib::commands::api::server_probe_fingerprint("127.0.0.1", relay_port)
            .await
            .expect("the fingerprint was not obtained through the relay");
    servers_api::server_fingerprint_confirm(state, server_id, &fingerprint)
        .expect("the fingerprint was not confirmed");
}

fn key_credentials() -> Credentials {
    Credentials::Key {
        path: key_path(),
        passphrase: Some(KEY_PASSPHRASE.to_owned()),
    }
}

/// A connection through the relay, the same way `ssh_live::connect` makes one directly.
async fn connect_via_relay(relay_port: u16) -> Connection {
    let addr = ServerAddress::new("127.0.0.1", relay_port);
    let fp = fingerprint::probe(&addr)
        .await
        .expect("the fingerprint was not obtained through the relay");
    Connection::connect(addr, "root", key_credentials(), &fp)
        .await
        .expect("connecting through the relay failed")
}

/// The pre-T562 shape, by hand: the same call `probed()` makes inside `build_from_server`
/// (`probe_moov::params_for` — read the head over SFTP, parse `moov`, write the result to the
/// database) for one file after another, on a fresh connection and a fresh, empty database so
/// neither side benefits from the other's warm-up or cache. This is the exact per-file cost
/// `build_from_server`'s loop used to pay one at a time before T562; `library_list`'s own
/// concurrent path must beat it, not merely equal it.
async fn measure_strictly_serial(relay_port: u16, names: &[String]) -> Duration {
    let conn = connect_via_relay(relay_port).await;
    let db = Db::open_in_memory().expect("the throwaway database would not open");
    let started = Instant::now();
    for name in names {
        probe_moov::params_for(
            &conn,
            &db,
            "throwaway-server",
            VIDEO_DIR,
            name,
            FILE_BYTES as u64,
        )
        .await
        .unwrap_or_else(|e| panic!("{name} would not probe over SFTP: {e}"));
    }
    let elapsed = started.elapsed();
    conn.close().await;
    elapsed
}

#[tokio::test]
async fn a_cold_library_of_64_files_is_probed_concurrently_and_faster_than_serial() {
    let server = TestServer::start().expect("the container would not come up");
    let names = file_names();
    prepare(&server, &names);

    let (relay_port, relay) = start_latency_relay(server.port).await;

    let state = app_state();
    let server_id = servers_api::server_add(&state, profile_via_relay(relay_port), KEY_PASSPHRASE)
        .expect("no profile");
    confirm_fingerprint_via_relay(&state, &server_id, relay_port).await;

    // The measured call itself: cold cache (a freshly added server has none), refresh forced.
    let started = Instant::now();
    let view = library_api::library_list(&state, &server_id, true)
        .await
        .expect("the library would not read");
    let concurrent_elapsed = started.elapsed();

    // Sanity first: a fast wrong answer is worse than a slow right one. If this fixture
    // stopped producing what the test thinks it does, the timing numbers below would be
    // meaningless.
    let media = view
        .media
        .iter()
        .find(|m| m.slug == "a-season")
        .expect("the medium did not appear in the library");
    assert_eq!(
        media.files.len(),
        FILE_COUNT,
        "not every file of the 64 made it into the medium — the timing below would not be \
         measuring what this test claims"
    );
    assert!(
        media.files.iter().all(|f| f.exists_on_server),
        "every file was just created on the server and must be seen as present"
    );

    println!(
        "library_scale: concurrent library_list(refresh=true) over {FILE_COUNT} files (relay \
         RTT ~{}ms) took {:.3}s",
        RELAY_ONE_WAY_DELAY.as_millis() * 2,
        concurrent_elapsed.as_secs_f64()
    );
    assert!(
        concurrent_elapsed < GENEROUS_CEILING,
        "library_list(refresh=true) over {FILE_COUNT} files took {concurrent_elapsed:?} — \
         longer than the generous ceiling of {GENEROUS_CEILING:?}. Either the SFTP probing \
         regressed to one round trip at a time, or something is hanging"
    );

    // The comparison baseline, measured after, through the same relay: sixty-odd probes done
    // one after another by hand — the exact shape the code had before T562.
    let serial_elapsed = measure_strictly_serial(relay_port, &names).await;
    println!(
        "library_scale: strictly serial equivalent over {FILE_COUNT} files took {:.3}s",
        serial_elapsed.as_secs_f64()
    );
    println!(
        "library_scale: ratio concurrent/serial = {:.3}",
        concurrent_elapsed.as_secs_f64() / serial_elapsed.as_secs_f64()
    );

    assert!(
        concurrent_elapsed < serial_elapsed,
        "library_list(refresh=true) ({concurrent_elapsed:?}) was not faster than probing the \
         same {FILE_COUNT} files one at a time by hand ({serial_elapsed:?}) over a link with \
         real round-trip delay — the concurrency T562 added is not actually taking effect"
    );

    relay.abort();
}

/// A small file count on purpose: exactly `BRIEF_CHANNELS` (6), so every probe in this test
/// lands in the same `.buffered()` batch and genuinely races the others — with more files
/// than that, a later one might not even start until the slow one has already finished, and
/// the ordering risk this test exists to catch would not be exercised.
const ORDER_FILE_COUNT: usize = 6;

/// The one file made deliberately far larger than the rest, so it demonstrably takes longer
/// to transfer through the relay — a real, measured difference in per-file completion time,
/// not a hoped-for scheduler accident. Still under `SUGGESTED_HEAD_BYTES` (512 KiB) so
/// `probed()` reads it whole in a single pass, same as the others.
const SLOW_FILE_BYTES: usize = 300 * 1024;

/// Its position among the six — not first, not last, so a naive "whatever finished last goes
/// last" bug would visibly move it rather than accidentally landing in the right place.
const SLOW_FILE_INDEX: usize = 2;

fn order_file_names() -> Vec<String> {
    (0..ORDER_FILE_COUNT)
        .map(|i| format!("Episode_{i:03}.mp4"))
        .collect()
}

fn prepare_with_one_slow_file(server: &TestServer, names: &[String]) {
    for (i, name) in names.iter().enumerate() {
        let bytes = if i == SLOW_FILE_INDEX {
            SLOW_FILE_BYTES
        } else {
            FILE_BYTES
        };
        server
            .exec_inside(&format!(
                "head -c {bytes} /dev/urandom > '{VIDEO_DIR}/{name}'"
            ))
            .unwrap_or_else(|e| panic!("could not create {name}: {e}"));
    }

    let files_json = names
        .iter()
        .map(|n| format!("\"{n}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let manifest = format!(
        r#"{{
      "generation": 1,
      "media": [
        {{ "id": "m_season", "title": "A Season", "slug": "a-season",
          "files": [{files_json}],
          "ladders": [],
          "created_at": "2026-08-01T10:00:00Z" }}
      ]
    }}"#
    );
    server
        .exec_inside(&format!(
            "cat > '{VIDEO_DIR}/library.json' <<'EOF'\n{manifest}\nEOF"
        ))
        .expect("could not write the catalogue");
}

/// T562(а)'s own explicit requirement: "порядок элементов в итоговом списке должен
/// сохраниться таким же... media/ladders не должны перемешаться местами при параллельном
/// ожидании, даже если запросы уходят одновременно." One file here is made to finish
/// noticeably later than the rest (see `SLOW_FILE_BYTES`); if `build_from_server` used
/// `buffer_unordered` instead of `buffered`, that slow file would be the last one *returned*
/// regardless of where it started, and this test would catch it moving.
#[tokio::test]
async fn a_slow_file_among_fast_ones_does_not_jump_the_queue() {
    let server = TestServer::start().expect("the container would not come up");
    let names = order_file_names();
    prepare_with_one_slow_file(&server, &names);

    let (relay_port, relay) = start_latency_relay(server.port).await;

    let state = app_state();
    let server_id = servers_api::server_add(&state, profile_via_relay(relay_port), KEY_PASSPHRASE)
        .expect("no profile");
    confirm_fingerprint_via_relay(&state, &server_id, relay_port).await;

    let view = library_api::library_list(&state, &server_id, true)
        .await
        .expect("the library would not read");
    let media = view
        .media
        .iter()
        .find(|m| m.slug == "a-season")
        .expect("the medium did not appear in the library");

    let expected_order: Vec<&str> = names.iter().map(String::as_str).collect();
    let actual_order: Vec<&str> = media.files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(
        actual_order, expected_order,
        "the slow file (index {SLOW_FILE_INDEX}, {SLOW_FILE_BYTES} bytes) changed the files' \
         order relative to the catalogue — concurrency must preserve order (buffered, not \
         buffer_unordered)"
    );

    relay.abort();
}
