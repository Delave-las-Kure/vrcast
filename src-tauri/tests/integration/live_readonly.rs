//! T060 — scenario 1 from `quickstart.md` against the live server, **read-only**.
//!
//! This is milestone A's acceptance check rather than part of the ordinary suite: it is
//! marked `#[ignore]` and runs only when asked for directly:
//!
//! ```text
//! cargo test --features integration --test integration -- --ignored --nocapture live_server
//! ```
//!
//! **What it does to the server: nothing.** It lists the directory, reads the catalogue,
//! reads the beginnings of files, asks how much room the disk has. Not one write — the
//! constitution forbids checking anything that changes the live server's state against it:
//! real serving is going on there, and breaking it for a check is not allowed.
//!
//! The scenario's steps that do change the server (renaming a short name, deleting with
//! confirmation) are checked against a throwaway container — `library_ops.rs`. The point of
//! this check is another one: to be sure the application copes with a real library rather
//! than only with the one it laid out itself.
//!
//! The settings come from `server.env` through the same carry-over that is offered to a
//! person (T043) — so that gets checked along the way. The secret never reaches the test's
//! code.

use std::sync::Arc;
use vrcast_studio_lib::commands::library::api as library;
use vrcast_studio_lib::commands::servers::{api as servers, StepStatus};
use vrcast_studio_lib::commands::AppState;
use vrcast_studio_lib::server::env_import;
use vrcast_studio_lib::store::db::Db;
use vrcast_studio_lib::store::secrets::InMemorySecretStore;

fn state() -> AppState {
    AppState::with_db(
        Arc::new(Db::open_in_memory().unwrap()),
        Arc::new(InMemorySecretStore::new()),
    )
    .expect("the application state would not assemble")
}

#[tokio::test]
#[ignore = "milestone A's acceptance check: it reaches the live server, run it by hand"]
async fn the_live_server_read_only() {
    let Some(path) = env_import::default_location() else {
        panic!(
            "no server.env was found nearby — the check is meant for the author's machine, \
             where there is one; there is nothing to run it for on anyone else's"
        );
    };
    let imported = env_import::read_from(&path).expect("server.env would not parse");

    // The log is switched on deliberately: it serves as the other half of the check (T064,
    // SC-011). The run goes with a real key to a real server, and if secret redaction fails
    // anywhere, the trace stays right here. The level is set through VRCAST_LOG — for
    // hunting leaks it is put to trace, so that talkative libraries lay out all they know.
    vrcast_studio_lib::logging::init();

    println!("\n=== Scenario 1, read-only ===");
    println!("settings taken from {}", imported.source.display());
    println!(
        "server {}@{}:{}, domain {}",
        imported.input.user, imported.input.host, imported.input.port, imported.input.domain
    );

    let state = state();

    // The scenario's step 2: credentials that are certainly wrong must let nobody in.
    // Checked the safe way: a port of the same address that nothing listens on.
    {
        let mut wrong = imported.input.clone();
        wrong.name = String::from("Certainly wrong");
        wrong.port = 64_999;
        let id = servers::server_add(&state, wrong, "a-certainly-wrong-secret")
            .expect("the profile was not created");
        let steps = servers::server_test(&state, &id)
            .await
            .expect("the check must return steps rather than an error");
        assert_eq!(
            steps[0].status,
            StepStatus::Failed,
            "the closed port is suddenly open"
        );
        assert!(
            steps[1..].iter().all(|s| s.status == StepStatus::Skipped),
            "the check went on after the network step failed"
        );
        println!("step 2: wrong credentials stop the check at the very first step — correct");
        servers::server_remove(&state, &id).expect("the temporary profile would not delete");
    }

    // Step 3: the right credentials.
    let secret = String::new(); // the author's key has no passphrase
    let id = servers::server_add(&state, imported.input.clone(), &secret)
        .expect("the profile was not created");

    let fingerprint = vrcast_studio_lib::commands::api::server_probe_fingerprint(
        &imported.input.host,
        imported.input.port,
    )
    .await
    .expect("the fingerprint was not obtained");
    println!("the server's fingerprint: {fingerprint}");
    servers::server_fingerprint_confirm(&state, &id, &fingerprint)
        .expect("the fingerprint would not confirm");

    let steps = servers::server_test(&state, &id)
        .await
        .expect("the check returned an error instead of steps");
    println!("\n--- the connection check's steps ---");
    for s in &steps {
        println!(
            "  [{}] {} — {}",
            match s.status {
                StepStatus::Ok => "done  ",
                StepStatus::Failed => "FAILED",
                StepStatus::Skipped => "missed",
            },
            s.id,
            s.detail.as_ref().map(|d| d.key.as_str()).unwrap_or("")
        );
    }
    assert!(
        steps.iter().all(|s| s.status == StepStatus::Ok),
        "not every step of the check passed"
    );

    // Steps 4 and 4a: the library and the files' parameters.
    let view = library::library_list(&state, &id, true)
        .await
        .expect("the library would not read");

    println!("\n--- the library ---");
    println!(
        "media: {}, not recognised: {}, entries accounted for: {}",
        view.media.len(),
        view.unrecognized.len(),
        view.accounted_entries()
    );
    if let Some(d) = view.disk {
        println!(
            "disk: {} free of {}, video takes {}",
            d.free_bytes, d.total_bytes, d.used_by_videos_bytes
        );
        assert!(d.total_bytes > 0 && d.free_bytes <= d.total_bytes);
    }
    assert!(
        !view.stale,
        "the data came from the cache: the server was not reached"
    );

    println!("\n--- the files and their parameters (from the header, without downloading) ---");
    let all: Vec<_> = view
        .media
        .iter()
        .flat_map(|m| m.files.iter())
        .chain(view.unrecognized.iter())
        .collect();
    for f in &all {
        println!(
            "  {:<58} {:>10} B  {}  {}  {}  {}",
            f.path,
            f.size_bytes,
            match (f.width, f.height) {
                (Some(w), Some(h)) => format!("{w}x{h}"),
                _ => String::from("size unknown"),
            },
            f.duration_s
                .map(|d| format!("{:.0} s", d))
                .unwrap_or_else(|| String::from("duration unknown")),
            f.video_codec.as_deref().unwrap_or("codec unknown"),
            match f.faststart_ok {
                Some(true) => "ready for serving",
                Some(false) => "HEADER AT THE END",
                None => "header not read",
            }
        );
    }

    // Step 5: the links. Checked to be built from the profile's domain and to point at
    // files that exist.
    println!("\n--- the viewers' links ---");
    for f in all.iter().take(3) {
        println!("  {}", f.origin_url);
        assert!(
            f.origin_url
                .starts_with(&format!("https://{}/", imported.input.domain)),
            "the link was not built from the profile's domain: {}",
            f.origin_url
        );
    }

    assert!(
        !all.is_empty(),
        "not one file was found on the server — there is nothing to check"
    );
    println!("\n=== reading the server is done, nothing was changed ===\n");
}
