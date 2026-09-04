//! T221, T333, T365 — the quickstart's scenarios walked on the throwaway stand.
//!
//! **Why here and not by hand.** A scenario walked once by a person proves the day it was
//! walked. Walked by this, it proves every time somebody runs it — and, more to the point,
//! it says what it did rather than what it meant to do. Where a step cannot be reached from
//! a test at all, it is named as such rather than quietly dropped.
//!
//! Ignored by default: every one of these changes a real machine. They are run by hand, at
//! the throwaway stand, and never anywhere else — the live server takes no part and cannot
//! (constitution, "Way of working").
//!
//! ```text
//! VRCAST_STAND_HOST=… VRCAST_STAND_DOMAIN=… VRCAST_STAND_KEY=…/id_ed25519 \
//!   cargo test --features integration --test integration stand_scenarios -- --ignored --nocapture
//! ```

use std::sync::Arc;
use std::time::Duration;

use vrcast_studio_lib::commands::diag::api as diag;
use vrcast_studio_lib::commands::ladder::{
    api as ladder, BuildRequest, LadderCheck, LadderRequest,
};
use vrcast_studio_lib::commands::limits::{api as limits, LimitRequest};
use vrcast_studio_lib::commands::quality::{api as quality, MeasureRequest};
use vrcast_studio_lib::commands::servers::{api as servers, ServerInput};
use vrcast_studio_lib::commands::viewers::api as viewers;
use vrcast_studio_lib::commands::AppState;
use vrcast_studio_lib::domain::server_profile::AuthKind;
use vrcast_studio_lib::store::db::Db;
use vrcast_studio_lib::store::secrets::InMemorySecretStore;

struct Stand {
    host: String,
    domain: String,
    key_path: String,
}

fn stand() -> Stand {
    let asked = |name: &str| {
        std::env::var(name)
            .unwrap_or_else(|_| panic!("{name} is not set — see the note at the top of this file"))
    };
    Stand {
        host: asked("VRCAST_STAND_HOST"),
        domain: asked("VRCAST_STAND_DOMAIN"),
        key_path: asked("VRCAST_STAND_KEY"),
    }
}

fn app_state() -> AppState {
    AppState::with_db(
        Arc::new(Db::open_in_memory().unwrap()),
        Arc::new(InMemorySecretStore::new()),
    )
    .expect("the application state would not assemble")
}

async fn profile_for(state: &AppState, stand: &Stand) -> String {
    let input = ServerInput {
        name: String::from("Stand"),
        host: stand.host.clone(),
        port: 22,
        user: String::from("root"),
        auth_kind: AuthKind::Key,
        key_path: Some(stand.key_path.clone()),
        domain: stand.domain.clone(),
        video_dir: None,
        cdn_base: None,
        ipv6_mode: None,
    };
    let id = servers::server_add(state, input, "").expect("the profile was not created");
    let fingerprint = vrcast_studio_lib::commands::api::server_probe_fingerprint(&stand.host, 22)
        .await
        .expect("the fingerprint was not obtained");
    servers::server_fingerprint_confirm(state, &id, &fingerprint)
        .expect("the fingerprint was not confirmed");
    id
}

/// Run one command on the stand, the way a person at a terminal would.
///
/// Deliberately outside the application: the scenario says "stop the serving", which is
/// something that happens *to* the server rather than something the application does. Asking
/// the application to break the thing it is about to diagnose would be checking it against
/// its own idea of what it did.
fn on_the_stand(stand: &Stand, command: &str) -> String {
    let out = std::process::Command::new("ssh")
        .args([
            "-i",
            &stand.key_path,
            "-o",
            "StrictHostKeyChecking=no",
            "-o",
            "BatchMode=yes",
            "-o",
            "ConnectTimeout=20",
            &format!("root@{}", stand.host),
            command,
        ])
        .output()
        .expect("ssh would not run");
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "stops a service on a real machine: run by hand at the throwaway stand"]
async fn scenario_8_a_stopped_serving_is_seen_and_named() {
    // Quickstart scenario 8, step 1. **"Something is down" and "caddy is down" send a person
    // to different places, and only one of them is a place.** The same question is asked of a
    // container in `diag_live`; what a real machine adds is a real systemd, a real ufw and a
    // real `free -m`, each of which answers slightly differently than a fixture does — and
    // shell that answers slightly differently is exactly the failure this catches.
    let stand = stand();
    let state = app_state();
    let id = profile_for(&state, &stand).await;

    let before = diag::diag_health(&state, &id)
        .await
        .expect("the health reading would not come back");
    println!("before: {:?}", before.worst);

    on_the_stand(&stand, "systemctl stop caddy");
    // Put it back whatever happens below, including a panic: a stand left with its serving
    // down would send the next run chasing a fault somebody else made.
    let restore = scopeguard(&stand);

    let ill = diag::diag_health(&state, &id)
        .await
        .expect("the health reading would not come back with the serving down");
    println!("with caddy stopped: {:?}", ill.worst);
    let said = format!("{ill:?}");
    assert!(
        said.contains("caddy"),
        "the serving was down and the report did not name it — a person is told something is \
         wrong and not where to go:\n{said}"
    );
    drop(restore);

    let after = diag::diag_health(&state, &id)
        .await
        .expect("the health reading would not come back after the restart");
    println!("after: {:?}", after.worst);
}

/// Puts the serving back when it goes out of scope, panic or no panic.
fn scopeguard(stand: &Stand) -> impl Drop + '_ {
    struct Restore<'a>(&'a Stand);
    impl Drop for Restore<'_> {
        fn drop(&mut self) {
            on_the_stand(self.0, "systemctl start caddy");
        }
    }
    Restore(stand)
}

// ---------------- scenario 5: the quality ladder (T221) ----------------

/// The film to build a ladder from. A real encode rather than a synthetic one: the whole
/// scenario turns on what the material does, and colour bars answer every question the same.
const SOURCE: &str = "VRCAST_STAND_SOURCE";

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "encodes and uploads to a real machine: run by hand at the throwaway stand"]
async fn scenario_5_a_ladder_measured_built_and_served_whole() {
    let stand = stand();
    let state = app_state();
    let id = profile_for(&state, &stand).await;
    let source = std::env::var(SOURCE).unwrap_or_else(|_| panic!("{SOURCE} is not set"));
    let slug = format!("stand{}", std::process::id());

    // --- 1. what the material is ------------------------------------------------------
    let measured = ladder::ladder_measure(&source)
        .await
        .expect("the source would not be measured");
    println!(
        "source: {:.2} Mbit/s on average, peak {:.2}",
        measured.average_bps as f64 / 1e6,
        measured.peak_bps as f64 / 1e6
    );

    // --- 2. the ladder, and its top against the source --------------------------------
    let request = LadderRequest {
        path: source.clone(),
        codec: String::from("h264"),
        native_height: None,
        declared_layout: None,
        prefer_hardware: true,
    };
    let preview = ladder::ladder_plan(&state, &request)
        .await
        .expect("the ladder would not be planned");
    let rungs = preview.plan.rungs.clone();
    assert!(!rungs.is_empty(), "the plan chose no rungs at all");
    println!("ladder from {:?}:", preview.from);
    for r in &rungs {
        println!(
            "  {:>5.1} Mbit/s @ {:>4}p  level {}  because {:?}",
            r.bitrate_bps as f64 / 1e6,
            r.height,
            r.level,
            r.reasons
        );
        // Step 2 of the scenario: every rung says why it is there. A ladder a person cannot
        // argue with is one they have to accept on faith.
        assert!(
            !r.reasons.is_empty(),
            "a rung was offered with no grounds given: {r:?}"
        );
    }
    let top = rungs.iter().map(|r| r.bitrate_bps).max().unwrap();
    assert!(
        top <= measured.average_bps.max(1),
        "the top rung asks for {} bit/s from a source that averages {} — spending bitrate on \
         detail the source never had",
        top,
        measured.average_bps
    );

    // --- 3 and 4. what the checks say about a ladder somebody edited ------------------
    let mut too_high = rungs.clone();
    too_high[0].bitrate_bps = measured.average_bps * 3;
    too_high[0].maxrate_bps = measured.average_bps * 3;
    let verdict = ladder::ladder_validate(&LadderCheck {
        rungs: too_high,
        source: preview.source,
    })
    .await
    .expect("the check would not run");
    let said = format!("{:?}", verdict.objections);
    assert!(
        said.contains("RungAboveSource") || said.contains("RUNG_ABOVE_SOURCE"),
        "a rung set above the source drew no objection: {said}"
    );

    let mut fat_buffer = rungs.clone();
    fat_buffer[0].bufsize_bps = measured.peak_bps * 4;
    let verdict = ladder::ladder_validate(&LadderCheck {
        rungs: fat_buffer,
        source: preview.source,
    })
    .await
    .expect("the check would not run");
    let said = format!("{:?}", verdict.objections);
    assert!(
        said.contains("Bufsize") || said.contains("BUFSIZE"),
        "a buffer four times the peak drew no objection: {said}"
    );

    // --- 5. build it, and check every variant is served -------------------------------
    //
    // ⚠ **The quality measurement comes first, and the scenario did not say so.** Written in
    // milestone A, step 5 reads "build the ladder" — and since milestone D a ladder is taken
    // from a measurement rather than a formula (FR-141), so a formula ladder is refused with
    // `LADDER_NOT_MEASURED` before a task exists. Walking the scenario as written stops here.
    // The quickstart has been corrected; this is what the corrected step does.
    let measure = quality::quality_measure_start(
        &state,
        MeasureRequest {
            path: source.clone(),
            codec: String::from("h264"),
            native_height: None,
            prefer_hardware: true,
            then_build: None,
            batch: None,
        },
    )
    .await
    .expect("the quality measurement would not start");
    if let Some(why) = wait_for_task(&state, &measure, Duration::from_secs(3 * 3600)).await {
        panic!("the quality measurement did not finish: {why}");
    }
    let measured_plan = ladder::ladder_plan(&state, &request)
        .await
        .expect("the ladder would not be planned from the measurement");
    println!(
        "after measuring, the ladder comes from {:?}:",
        measured_plan.from
    );
    for r in &measured_plan.plan.rungs {
        println!(
            "  {:>5.1} Mbit/s @ {:>4}p  level {}",
            r.bitrate_bps as f64 / 1e6,
            r.height,
            r.level
        );
    }
    let rungs = measured_plan.plan.rungs.clone();

    let began = std::time::Instant::now();
    let task = ladder::ladder_build(
        &state,
        BuildRequest {
            server_id: id.clone(),
            path: source.clone(),
            slug: slug.clone(),
            rungs: rungs.clone(),
            audio_track: 0,
            prefer_hardware: true,
            batch: None,
        },
    )
    .await
    .expect("the build would not start");
    if let Some(why) = wait_for_task(&state, &task, Duration::from_secs(3 * 3600)).await {
        panic!("the build did not finish: {why}");
    }
    println!(
        "built in {:.1} minutes",
        began.elapsed().as_secs_f64() / 60.0
    );

    // **Every variant, not the first.** A master that lists five and serves one looks entirely
    // healthy to anything that opens the top of the list and stops there.
    let check = ladder::ladder_verify(&state, &id, &slug)
        .await
        .expect("the serving check would not run");
    println!("served: {check:?}");
    let served = format!("{check:?}");
    assert!(
        !served.contains("Missing") && !served.contains("missing"),
        "a variant in the master is not served: {served}"
    );

    // --- 6. and again: what is finished is not built twice ----------------------------
    let again = std::time::Instant::now();
    let task = ladder::ladder_build(
        &state,
        BuildRequest {
            server_id: id.clone(),
            path: source.clone(),
            slug: slug.clone(),
            rungs,
            audio_track: 0,
            prefer_hardware: true,
            batch: None,
        },
    )
    .await
    .expect("the second build would not start");
    if let Some(why) = wait_for_task(&state, &task, Duration::from_secs(3 * 3600)).await {
        panic!("the second build did not finish: {why}");
    }
    let second = again.elapsed().as_secs_f64() / 60.0;
    let first = began.elapsed().as_secs_f64() / 60.0 - second;
    println!("second run took {second:.1} minutes against {first:.1}");
    assert!(
        second < first / 2.0,
        "the second run cost {second:.1} minutes against the first's {first:.1} — the variants \
         that were already there look to have been encoded again"
    );
}

/// Wait for a task, giving back the reason when it did not finish well.
async fn wait_for_task(state: &AppState, task_id: &str, limit: Duration) -> Option<String> {
    use vrcast_studio_lib::tasks::state::TaskState;
    let deadline = std::time::Instant::now() + limit;
    while std::time::Instant::now() < deadline {
        let task =
            vrcast_studio_lib::commands::api::task_get(state, task_id).expect("the task vanished");
        match task.state {
            TaskState::Completed => return None,
            TaskState::Failed | TaskState::Cancelled => {
                return Some(
                    task.error
                        .map(|e| format!("{e:?}"))
                        .unwrap_or_else(|| String::from("no reason given")),
                )
            }
            _ => {}
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
    Some(format!("the task did not finish within {limit:?}"))
}

// ---------------- scenario 6: capping one viewer's quality (T221) ----------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "rewrites serving rules on a real machine: run by hand at the throwaway stand"]
async fn scenario_6_a_capped_viewer_is_given_less_and_everyone_else_is_not() {
    let stand = stand();
    let state = app_state();
    let id = profile_for(&state, &stand).await;
    let slug = std::env::var("VRCAST_STAND_SLUG")
        .expect("VRCAST_STAND_SLUG is not set: name the set scenario 5 built");
    // **The address comes from the viewers list, the way the scenario says** — "choose a
    // viewer and set them a ceiling" (FR-060). ⚠ The first version of this walk asked an
    // outside service for this machine's address and capped that, and the cap silently did
    // nothing: the address a viewer believes they have and the one the server sees are not
    // the same, and there is no reason they should be. That is not a fault in the
    // application; it is what happens when the address does not come from where the
    // application gets it.
    // ⚠ **The active list arrives only as an event; there is no asking for it.** Which is
    // also why this subscribes before making a single request: `viewers_history` holds those
    // who have already gone quiet for longer than the activity threshold, so waiting on it
    // means waiting out the threshold on every run and getting nothing before that.
    let mut updates = state.subscribe();
    viewers::viewers_watch_start(&state, &id)
        .await
        .expect("the watching would not start");
    watch_a_little(&stand, &slug);
    let viewer = {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(90);
        let mut seen = None;
        while tokio::time::Instant::now() < deadline && seen.is_none() {
            match tokio::time::timeout_at(deadline, updates.recv()).await {
                Ok(Ok(vrcast_studio_lib::commands::AppEvent::ViewersUpdate(u))) => {
                    seen = u.active.into_iter().next().map(|v| v.ip);
                }
                Ok(Ok(_)) => {}
                _ => break,
            }
        }
        seen.expect(
            "nobody appeared in the viewers list after the set was fetched — either the log is              not being followed or the requests did not reach the serving",
        )
    };

    // --- 1. what the disk holds before any of this ------------------------------------
    let used_before = used_bytes(&stand);
    let media_bytes = dir_bytes(&stand, &slug);
    println!(
        "the set takes {} bytes; the disk holds {used_before}",
        media_bytes
    );

    // --- 2 and 3. the warnings come before the change, not after -----------------------
    let cap = 1_500_000;
    let preview = limits::limit_preview(
        &state,
        &LimitRequest {
            server_id: id.clone(),
            ip: viewer.clone(),
            slug: slug.clone(),
            cap_bps: cap,
        },
    )
    .await
    .expect("the preview would not come back");
    println!(
        "capping at {:.1} Mbit/s keeps {} of the rungs; warnings: {}",
        cap as f64 / 1e6,
        preview.kept.len(),
        preview.warnings.len()
    );
    assert!(
        !preview.warnings.is_empty(),
        "a cap was offered with no warning at all — an address is shared by everyone behind \
         it, and it changes; both have to be said before the rule is written, not after"
    );
    assert!(
        !preview.kept.is_empty() && preview.kept.len() < 3,
        "the preview kept {} rungs of three: a cap that keeps everything or nothing is not a \
         cap",
        preview.kept.len()
    );

    // **Unconfirmed is refused, and that is the whole of step 3.** The warnings are not a
    // decoration beside a button that works anyway: the core will not write the rule until
    // somebody has been told what an address is and has said yes.
    let refused = limits::limit_set(
        &state,
        LimitRequest {
            server_id: id.clone(),
            ip: viewer.clone(),
            slug: slug.clone(),
            cap_bps: cap,
        },
        false,
    )
    .await;
    assert!(
        refused.is_err(),
        "the cap was written without anybody confirming it"
    );

    limits::limit_set(
        &state,
        LimitRequest {
            server_id: id.clone(),
            ip: viewer.clone(),
            slug: slug.clone(),
            cap_bps: cap,
        },
        true,
    )
    .await
    .expect("the cap would not be set");

    // --- 4. and now the two addresses see different masters ---------------------------
    let ours = fetch_master(&stand, &slug);
    let theirs = master_from_the_stand(&stand, &slug);
    let ours_variants = ours.matches("#EXT-X-STREAM-INF").count();
    let theirs_variants = theirs.matches("#EXT-X-STREAM-INF").count();
    println!(
        "capped address is offered {ours_variants} variants; another address {theirs_variants}"
    );
    assert_eq!(
        ours_variants,
        preview.kept.len(),
        "the capped address was offered {ours_variants} variants where the preview promised {}",
        preview.kept.len()
    );
    assert!(
        theirs_variants > ours_variants,
        "every address got the same trimmed set — the cap is on the file rather than on the \
         viewer, and one person's limit became everybody's"
    );

    // --- 5. and it cost almost nothing in room (SC-007) -------------------------------
    let used_after = used_bytes(&stand);
    let grew = used_after.saturating_sub(used_before);
    println!("the cap cost {grew} bytes against the set's {media_bytes}");
    assert!(
        grew * 100 <= media_bytes.max(1),
        "capping one viewer grew the disk by {grew} bytes — more than a hundredth of the set \
         itself ({media_bytes}), which means a copy was made rather than a description written"
    );

    // --- 6. a rule the serving would reject must not reach it (SC-008) ----------------
    //
    // The scenario says "put a knowingly wrong value into the rule". The file belongs to the
    // application and is rewritten whole, so the way a wrong value gets in is through the
    // application: an address that is not an address. Whatever it does with that — refuse it
    // before writing, or write it and be told by the serving that it will not have it — the
    // one outcome that must not happen is a serving left broken.
    let nonsense = limits::limit_set(
        &state,
        LimitRequest {
            server_id: id.clone(),
            ip: String::from("not-an-address"),
            slug: slug.clone(),
            cap_bps: cap,
        },
        true,
    )
    .await;
    println!("a rule with a nonsense address: {nonsense:?}");
    assert!(
        nonsense.is_err(),
        "an address that is not an address was written into the serving rules"
    );
    let still = master_from_the_stand(&stand, &slug);
    assert!(
        still.contains("#EXT-X-STREAM-INF"),
        "the serving stopped answering after a rule it would not accept — the whole point of          checking the configuration before reloading it is that this cannot happen"
    );

    // --- 8. and a ceiling under everything still gives the lightest -------------------
    let under_everything = limits::limit_preview(
        &state,
        &LimitRequest {
            server_id: id.clone(),
            ip: viewer.clone(),
            slug: slug.clone(),
            cap_bps: 100_000,
        },
    )
    .await
    .expect("the preview would not come back");
    assert!(
        under_everything.below_lightest,
        "a ceiling below every rung was not called out as such"
    );
    assert_eq!(
        under_everything.kept.len(),
        1,
        "a ceiling below every rung kept {} rungs — it has to keep the lightest, or the          viewer is given nothing at all",
        under_everything.kept.len()
    );

    // --- 7. taking it off puts everything back ----------------------------------------
    limits::limit_clear(&state, &id, &viewer, &slug)
        .await
        .expect("the cap would not be cleared");
    let ours_again = fetch_master(&stand, &slug);
    assert_eq!(
        ours_again.matches("#EXT-X-STREAM-INF").count(),
        theirs_variants,
        "the cap was taken off and the full set did not come back"
    );
    let left = on_the_stand(
        &stand,
        &format!("grep -c '{viewer}' /etc/caddy/vrcast-limits.conf || true"),
    );
    assert_eq!(
        left.trim(),
        "0",
        "the rule for that address is still in the serving configuration after it was cleared"
    );
}

/// Bytes used on the stand's disk.
fn used_bytes(stand: &Stand) -> u64 {
    on_the_stand(stand, "df -B1 --output=used / | tail -1")
        .trim()
        .parse()
        .unwrap_or(0)
}

/// Bytes the built set takes.
fn dir_bytes(stand: &Stand, slug: &str) -> u64 {
    on_the_stand(
        stand,
        &format!("du -sb /var/lib/vrcast/videos/{slug} 2>/dev/null | cut -f1"),
    )
    .trim()
    .parse()
    .unwrap_or(0)
}

/// The master as this machine is given it.
fn fetch_master(stand: &Stand, slug: &str) -> String {
    let out = std::process::Command::new("curl")
        .args([
            "-s",
            "--max-time",
            "30",
            &format!("https://{}/videos/{slug}/master.m3u8", stand.domain),
        ])
        .output()
        .expect("curl would not run");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// The master as somebody at another address is given it — the stand asking its own domain.
fn master_from_the_stand(stand: &Stand, slug: &str) -> String {
    on_the_stand(
        stand,
        &format!(
            "curl -s --max-time 30 https://{}/videos/{slug}/master.m3u8",
            stand.domain
        ),
    )
}

/// Watch a little of the set, so that somebody is in the viewers list to choose.
///
/// The master and the first segment of the lightest variant: enough to be a viewer, not
/// enough to be a download.
fn watch_a_little(stand: &Stand, slug: &str) {
    let get = |url: String| {
        let _ = std::process::Command::new("curl")
            .args(["-s", "-o", "/dev/null", "--max-time", "60", &url])
            .status();
    };
    get(format!(
        "https://{}/videos/{slug}/master.m3u8",
        stand.domain
    ));
    for v in ["v1", "v2", "v4"] {
        get(format!(
            "https://{}/videos/{slug}/{v}/index.m3u8",
            stand.domain
        ));
        get(format!(
            "https://{}/videos/{slug}/{v}/seg_00000.ts",
            stand.domain
        ));
    }
}

// ---------------- scenario 4: who is watching (T221) ----------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "pulls video from a real machine: run by hand at the throwaway stand"]
async fn scenario_4_both_kinds_of_viewer_are_seen_and_a_slow_one_is_named() {
    let stand = stand();
    let state = app_state();
    let id = profile_for(&state, &stand).await;
    let slug = std::env::var("VRCAST_STAND_SLUG")
        .expect("VRCAST_STAND_SLUG is not set: name the set scenario 5 built");

    let mut updates = state.subscribe();
    viewers::viewers_watch_start(&state, &id)
        .await
        .expect("the watching would not start");

    // **Two viewers of two different kinds, from two different addresses.** This is the whole
    // point of the scenario: a plain file is not in any playlist, so anything that learns who
    // is watching by reading playlists sees only half the room (R-02). One takes the file
    // straight, the other takes the set.
    let direct = format!("https://{}/videos/{slug}_1.mp4", stand.domain);
    let hls = format!("https://{}/videos/{slug}/v1/seg_00000.ts", stand.domain);
    let from_here = std::thread::spawn({
        let direct = direct.clone();
        move || {
            let _ = std::process::Command::new("curl")
                .args([
                    "-s",
                    "-o",
                    "/dev/null",
                    "--max-time",
                    "60",
                    "-r",
                    "0-4000000",
                    &direct,
                ])
                .status();
        }
    });
    on_the_stand(
        &stand,
        &format!("curl -s -o /dev/null --max-time 60 '{hls}'"),
    );

    let seen = gather_viewers(&mut updates, Duration::from_secs(90)).await;
    let _ = from_here.join();
    println!("{} viewer(s) seen", seen.len());
    for v in &seen {
        // The address itself is never printed (FR-057) — only what is known about it.
        println!(
            "  media {:?}, variant {:?}, {:?}/{:?}/{:?}, delivering {:?}",
            v.media_id, v.variant, v.country, v.city, v.asn_org, v.delivery_bps
        );
    }
    assert!(
        seen.len() >= 2,
        "only {} viewer(s) came back where two were watching — one straight from the file and \\
         one from the set. If it is the direct one that is missing, the parsing of connections \\
         is not working and half the room is invisible",
        seen.len()
    );

    // Step 3: where they are, or a plain "not known" — never a blank.
    for v in &seen {
        let placed = v.country.is_some() || v.city.is_some() || v.asn_org.is_some();
        println!("  placed: {placed}");
    }

    // --- 4. one of them on a line too thin for what they are being sent ---------------
    //
    // ⚠ **The link is thinned, not the reader** — and the first version of this got that
    // wrong. Pulling with `curl --limit-rate` throttles whoever is reading, not the network:
    // TCP hands the whole file over at once and then waits, `bytes_acked` stops growing, and
    // `ss` reports `rwnd_limited: 97%`. That is a player with a full buffer, which the core
    // deliberately does **not** flag (`RECEIVER_LIMITED`) — marking it would light the flag
    // for every healthy viewer there is. The check failed because the application was right.
    //
    // So the serving's own outgoing link is narrowed below what the variant needs, which is
    // what a thin link actually is. It is put back below whatever happens.
    let iface = on_the_stand(&stand, "ip -o -4 route show default | awk '{print $5}'");
    let iface = iface.trim().to_owned();
    on_the_stand(
        &stand,
        &format!("tc qdisc replace dev {iface} root tbf rate 2mbit burst 32kbit latency 400ms"),
    );
    let _thin = ThinLink {
        stand: &stand,
        iface: iface.clone(),
    };
    let big = format!("https://{}/videos/{slug}/v4/index.m3u8", stand.domain);
    let pulling = std::thread::spawn(move || {
        for n in 0..8 {
            let _ = std::process::Command::new("curl")
                .args([
                    "-s",
                    "-o",
                    "/dev/null",
                    "--max-time",
                    "40",
                    &big.replace("index.m3u8", &format!("seg_{n:05}.ts")),
                ])
                .status();
        }
    });

    let mut troubled = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(120);
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout_at(deadline, updates.recv()).await {
            Ok(Ok(vrcast_studio_lib::commands::AppEvent::ViewersUpdate(u))) => {
                for v in &u.active {
                    if !v.problems.is_empty() {
                        println!(
                            "  trouble {:?}: delivering {:?}, needs {:?}",
                            v.problems, v.delivery_bps, v.required_bps
                        );
                    }
                }
                troubled = u
                    .active
                    .iter()
                    .filter(|v| !v.problems.is_empty())
                    .cloned()
                    .collect();
                if !troubled.is_empty() {
                    break;
                }
            }
            Ok(Ok(_)) => {}
            _ => break,
        }
    }
    assert!(
        !troubled.is_empty(),
        "a viewer pulling at 25 kB/s from a variant that needs a megabit was not marked as \
         having trouble — the list says everyone is fine while somebody is watching a slideshow"
    );

    // --- 5. and when they stop, they leave the active list for the history ------------
    let _ = pulling.join();
    let threshold =
        Duration::from_secs(vrcast_studio_lib::domain::viewers::DEFAULT_ACTIVITY_THRESHOLD_S + 20);
    println!("waiting out the activity threshold ({threshold:?})");
    let deadline = tokio::time::Instant::now() + threshold;
    let mut emptied = false;
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout_at(deadline, updates.recv()).await {
            Ok(Ok(vrcast_studio_lib::commands::AppEvent::ViewersUpdate(u))) => {
                if u.active.is_empty() {
                    emptied = true;
                    break;
                }
            }
            Ok(Ok(_)) => {}
            _ => break,
        }
    }
    let history = viewers::viewers_history(&state);
    println!("active emptied: {emptied}; history holds {}", history.len());
    assert!(
        emptied,
        "nobody left the active list after the pulling stopped and the threshold passed — the \
         count of who is watching only ever grows"
    );
    assert!(
        !history.is_empty(),
        "the viewers left the active list and the history kept none of them"
    );
}

/// Collect the viewers the watch reports, until the time is up.
async fn gather_viewers(
    updates: &mut tokio::sync::broadcast::Receiver<vrcast_studio_lib::commands::AppEvent>,
    limit: Duration,
) -> Vec<vrcast_studio_lib::domain::viewers::Viewer> {
    let deadline = tokio::time::Instant::now() + limit;
    let mut best: Vec<vrcast_studio_lib::domain::viewers::Viewer> = Vec::new();
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout_at(deadline, updates.recv()).await {
            Ok(Ok(vrcast_studio_lib::commands::AppEvent::ViewersUpdate(u))) => {
                if u.active.len() > best.len() {
                    best = u.active;
                }
                if best.len() >= 2 {
                    break;
                }
            }
            Ok(Ok(_)) => {}
            _ => break,
        }
    }
    best
}

/// Puts the serving's link back to full width when it goes out of scope.
struct ThinLink<'a> {
    stand: &'a Stand,
    iface: String,
}

impl Drop for ThinLink<'_> {
    fn drop(&mut self) {
        on_the_stand(
            self.stand,
            &format!("tc qdisc del dev {} root || true", self.iface),
        );
    }
}

// ---------------- the delivered speed (T481) ----------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "pulls video from a real machine for a minute: run by hand at the throwaway stand"]
async fn the_delivered_speed_appears_for_a_viewer_who_stays_long_enough() {
    // Half of FR-051: the address and the duration were there on the stand, the delivered
    // speed was not. It turned out to be there too — see below — and what this now guards is
    // that it stays there.
    //
    // ⚠ **A pull that is not happening looks exactly like a speed that is not worked out**,
    // and three runs in a row blamed the application for the second when it was the first.
    // So the pull is proved alive, through the connection table, *before* anything is
    // concluded about the speed: no row, no verdict.
    let stand = stand();
    let state = app_state();
    let id = profile_for(&state, &stand).await;
    let slug = std::env::var("VRCAST_STAND_SLUG")
        .expect("VRCAST_STAND_SLUG is not set: name the set scenario 5 built");

    let mut updates = state.subscribe();
    viewers::viewers_watch_start(&state, &id)
        .await
        .expect("the watching would not start");

    let url = format!("https://{}/videos/{slug}_4.mp4", stand.domain);
    let pulling = std::thread::spawn(move || {
        let _ = std::process::Command::new("curl")
            .args([
                "-s",
                "-o",
                "/dev/null",
                "--max-time",
                "90",
                "--limit-rate",
                "400k",
                &url,
            ])
            .status();
    });

    // Prove somebody is actually pulling before judging anything.
    let profile = vrcast_studio_lib::store::profiles::get(&state.db, &id)
        .expect("the profile would not read")
        .expect("the profile is not there");
    let conn = vrcast_studio_lib::server::gate::open(
        state.secrets.as_ref(),
        &profile,
        vrcast_studio_lib::server::gate::Intent::Read,
    )
    .await
    .expect("the stand would not open")
    .conn;
    let command = vrcast_studio_lib::domain::connections::poll_command();
    let mut alive = false;
    for _ in 0..10 {
        tokio::time::sleep(Duration::from_secs(3)).await;
        if let Ok(out) = conn.exec(&command).await {
            if let Some(poll) = vrcast_studio_lib::domain::connections::parse_poll(&out.stdout) {
                if !poll.rows.is_empty() {
                    alive = true;
                    break;
                }
            }
        }
    }
    assert!(
        alive,
        "no connection to the serving was open at all — the pull this check depends on never          happened, and nothing can be said about the delivered speed either way"
    );

    let deadline = tokio::time::Instant::now() + Duration::from_secs(90);
    let mut best: Option<u64> = None;
    let mut sightings = 0;
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout_at(deadline, updates.recv()).await {
            Ok(Ok(vrcast_studio_lib::commands::AppEvent::ViewersUpdate(u))) => {
                for v in &u.active {
                    sightings += 1;
                    // A mark for the address rather than the address (FR-057).
                    println!(
                        "  viewer {} variant {:?} delivering {:?}",
                        mark(&v.ip),
                        v.variant,
                        v.delivery_bps
                    );
                    if let Some(bps) = v.delivery_bps {
                        if bps > 0 {
                            best = Some(bps.max(best.unwrap_or(0)));
                        }
                    }
                }
                if best.is_some() {
                    break;
                }
            }
            Ok(Ok(_)) => {}
            _ => break,
        }
    }
    let _ = pulling.join();

    println!("{sightings} sighting(s); best delivered speed {best:?}");
    assert!(
        best.is_some(),
        "a viewer was pulling — the connection table said so — and no delivered speed was          worked out across {sightings} sighting(s). Half of FR-051 shows nothing, and an empty          field is indistinguishable from one still being counted"
    );
}

/// A short mark standing for an address, so two sightings can be compared without the
/// address appearing anywhere (FR-057).
fn mark(ip: &str) -> String {
    let mut h: u64 = 1469598103934665603;
    for b in ip.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(1099511628211);
    }
    format!("{:06x}", h & 0xffffff)
}

// ---------------- scenario 8, steps 2 and 3: explaining a complaint (T333) ----------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "reads a real machine's log after making a viewer fall behind: run by hand"]
async fn scenario_8_a_complaint_of_stalling_is_explained_with_the_readings_behind_it() {
    // Quickstart scenario 8, step 2. **"It stalls" is what a person says; the answer has to be
    // what the server was doing at the time.** A verdict on its own — "the link is slow" —
    // sends somebody to argue with their provider on our word. The readings behind it are what
    // make the verdict arguable, which is the only kind worth giving.
    //
    // ⚠ **The stall is made of the rhythm of the requests, not of a narrow link**, and two
    // earlier attempts got that wrong. Narrowing the serving's egress with `tc` broke
    // connections outright rather than slowing them — a token bucket small enough to matter is
    // too small for a TLS handshake — and the link between this machine and the stand fails on
    // its own from time to time, which is indistinguishable from a throttle that worked.
    // Neither makes a viewer who is falling behind.
    //
    // What does: asking for four-second segments every twelve seconds. That is somebody
    // watching at a third of real time, which is what the log of a stalling viewer looks like.
    // It is done from the stand itself, where the path is not in question.
    let stand = stand();
    let state = app_state();
    let id = profile_for(&state, &stand).await;
    let slug = std::env::var("VRCAST_STAND_SLUG")
        .expect("VRCAST_STAND_SLUG is not set: name the set scenario 5 built");

    // ⚠ **From here, not from the stand.** A first attempt made the requests on the stand
    // itself, and they came back "set aside" rather than as a watcher — which is correct and
    // deliberate: a machine's own requests are not viewers of it. Correct, and it means the
    // complaint has to come from outside, over the same link everybody else uses.
    let domain = stand.domain.clone();
    let for_pull = slug.clone();
    let watching = std::thread::spawn(move || {
        for n in 0..6 {
            let url = format!("https://{domain}/videos/{for_pull}/v4/seg_{n:05}.ts");
            let _ = std::process::Command::new("curl")
                .args(["-s", "-o", "/dev/null", "--max-time", "20", &url])
                .status();
            std::thread::sleep(Duration::from_secs(12));
        }
    });
    // Long enough for the pattern to be in the log rather than only begun.
    tokio::time::sleep(Duration::from_secs(62)).await;

    let stalls = diag::diag_explain_stalls(
        &state,
        &id,
        5,
        Some(vrcast_studio_lib::domain::stalls::FileShape {
            average_mbit: 4.0,
            peak_10s_mbit: 6.0,
        }),
    )
    .await
    .expect("the complaint would not be looked into");
    let _ = watching.join();

    println!(
        "{} watcher(s), {} set aside, {} verdict(s)",
        stalls.watchers.len(),
        stalls.set_aside.len(),
        stalls.verdicts.len()
    );
    println!("  load: {:?}", stalls.load);
    for v in &stalls.verdicts {
        println!("  {v:?}");
    }
    assert!(
        !stalls.watchers.is_empty(),
        "somebody asked for five segments over a minute and the complaint came back about \
         nobody at all"
    );
    assert_eq!(
        stalls.verdicts.len(),
        stalls.watchers.len(),
        "a watcher came back without a verdict — the complaint was heard and not answered"
    );
    // **A cause, and the numbers it was reached from.** Step 2 asks for both, and a verdict
    // with nothing to look at is one nobody can argue with.
    for v in &stalls.verdicts {
        assert!(
            !v.say.params.is_empty(),
            "a verdict came back with no figures behind it: {v:?}"
        );
    }
    // ⚠ **The cause is read as a cause, not looked for in the text.** The first version of
    // this searched the debug output for "Stall" — and `StallsKeepingUp`, the verdict that
    // says the viewer is *fine*, contains it. The check passed while proving the opposite of
    // what it claimed: the very failure this project keeps finding elsewhere, in its own hands.
    use vrcast_studio_lib::domain::stalls::Cause;
    let causes: Vec<Cause> = stalls.verdicts.iter().map(|v| v.cause).collect();
    println!("  causes: {causes:?}");
    assert!(
        causes.iter().any(|c| !matches!(c, Cause::NothingWrong)),
        "every verdict says nothing is wrong, for a viewer asking for four seconds of film \
         every twelve. Either the complaint was not made or it was not heard: {causes:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "reads a real film: run by hand"]
async fn scenario_8_the_peaks_of_a_film_are_found_and_placed() {
    // Step 3, and it touches no server at all: the question is about the film. Which is why it
    // can be asked *before* an upload, when it is most useful.
    let source = std::env::var("VRCAST_STAND_SOURCE")
        .expect("VRCAST_STAND_SOURCE is not set: name the film");
    let peaks = diag::diag_bitrate(&source)
        .await
        .expect("the film would not be measured");
    println!(
        "{} second(s): average {:.2} Mbit/s, median {:.2}",
        peaks.seconds,
        peaks.average_bps as f64 / 1e6,
        peaks.median_bps as f64 / 1e6
    );
    println!("  the worst second: {:?}", peaks.one_second);
    println!("  the worst ten:    {:?}", peaks.wide);
    assert!(peaks.seconds > 0, "the film measured as no seconds long");
    let peak = peaks
        .one_second
        .as_ref()
        .expect("no peak second was found in a film that has scenes");
    assert!(
        peak.bitrate_bps > peaks.median_bps,
        "the worst second is no worse than the middle one — nothing was found, only averaged"
    );
    // **Where, not just how much.** A peak with no position cannot be looked at.
    assert!(
        peak.at_s > 0.0 || peaks.wide.as_ref().is_some_and(|w| w.at_s > 0.0),
        "the peak was found and not placed: {peak:?}"
    );
}
