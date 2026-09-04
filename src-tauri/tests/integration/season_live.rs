//! T447 — a real season through the application's own commands.
//!
//! **What this reaches that no unit check can.** Everything the loan and its check are built
//! on was measured with a shell script and then checked against numbers written into a test
//! file (`domain::check_point`). What has never happened is the application itself measuring
//! one episode, lending that measurement onward, and the check holding or refusing on
//! material nobody chose for the purpose. A rule proven on four numbers is proven about the
//! numbers, not about the season.
//!
//! Ignored by default: it runs for tens of minutes and needs a real encoder and real films.
//!
//! ```text
//! VRCAST_SEASON="F:/films/e01.mkv,F:/films/e02.mkv" \
//!   cargo test --features integration --test integration season -- --ignored --nocapture
//! ```
//!
//! The first film named is measured in full; every one after it borrows from the first and is
//! checked. What comes out is a table — rungs per episode, minutes per episode, and what the
//! check said. That table is the point: T447 asks for a reading, not an impression.

use std::sync::Arc;
use std::time::{Duration, Instant};

use vrcast_studio_lib::commands::quality::{api as quality, MeasureRequest};
use vrcast_studio_lib::commands::AppState;
use vrcast_studio_lib::store::db::Db;
use vrcast_studio_lib::store::secrets::InMemorySecretStore;

/// Where the season is. Absolute paths, comma-separated, the first one the donor.
const SEASON: &str = "VRCAST_SEASON";
/// The height the material really has, when it was upscaled. Told, never guessed.
const NATIVE_HEIGHT: &str = "VRCAST_SEASON_NATIVE_HEIGHT";

fn app_state() -> AppState {
    AppState::with_db(
        Arc::new(Db::open_in_memory().unwrap()),
        Arc::new(InMemorySecretStore::new()),
    )
    .expect("the application state would not assemble")
}

fn episodes() -> Vec<String> {
    let list = std::env::var(SEASON)
        .unwrap_or_else(|_| panic!("{SEASON} is not set: name the season, comma-separated"));
    let files: Vec<String> = list
        .split(',')
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .collect();
    assert!(
        files.len() >= 2,
        "a season of one film has nothing to lend to: {SEASON} needs at least two"
    );
    for f in &files {
        assert!(
            std::path::Path::new(f).is_file(),
            "no such file, or it is not a file: {f}"
        );
    }
    files
}

fn request(path: &str) -> MeasureRequest {
    MeasureRequest {
        path: path.to_owned(),
        codec: String::from("h264"),
        native_height: std::env::var(NATIVE_HEIGHT)
            .ok()
            .and_then(|s| s.parse().ok()),
        prefer_hardware: true,
        then_build: None,
        batch: None,
    }
}

/// Wait for a task, giving back the reason when it did not finish well.
async fn wait_for(state: &AppState, task_id: &str, limit: Duration) -> Option<String> {
    use vrcast_studio_lib::tasks::state::TaskState;
    let deadline = Instant::now() + limit;
    while Instant::now() < deadline {
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

/// One line of the reading this test exists to produce.
struct Line {
    film: String,
    rungs: usize,
    minutes: f64,
    verdict: String,
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs a real encoder and a real season: run by hand"]
async fn a_season_measured_once_and_checked_thereafter() {
    let films = episodes();
    let state = app_state();
    let mut table: Vec<Line> = Vec::new();

    // --- the donor, measured in full --------------------------------------------------
    let donor = &films[0];
    let began = Instant::now();
    let task = quality::quality_measure_start(&state, request(donor))
        .await
        .expect("the measurement would not start");
    if let Some(why) = wait_for(&state, &task, Duration::from_secs(3 * 3600)).await {
        panic!("the donor's measurement did not finish: {why}");
    }
    let donor_key = quality::quality_measurements(&state)
        .await
        .expect("the measurements would not list")
        .into_iter()
        .find(|r| &r.source_path == donor)
        .expect("the donor's measurement is not in the store")
        .source_key;
    let donor_view = quality::quality_measure_result(&state, &donor_key, "h264")
        .await
        .expect("the donor's result would not read");
    let donor_rungs = donor_view
        .selection
        .as_ref()
        .map(|s| s.rungs.len())
        .unwrap_or(0);
    println!(
        "\nmeasured at seconds {:?} ({} s each); {} points of the grid landed; anchor {} Mbit/s",
        donor_view.run.chunk_starts,
        donor_view.run.chunk_s,
        donor_view.points.len(),
        donor_view.run.anchor_mbps
    );
    if let Some(chosen) = donor_view.selection.as_ref() {
        println!("\nthe donor's ladder, heaviest first:");
        for rung in &chosen.rungs {
            println!(
                "  {:>3} Mbit/s @ {:>4}p  VMAF {:.2}{}",
                rung.bitrate_mbps,
                rung.height,
                rung.vmaf,
                if rung.filled_a_gap {
                    "  (fills a gap)"
                } else {
                    ""
                }
            );
        }
    }
    table.push(Line {
        film: name_of(donor),
        rungs: donor_rungs,
        minutes: began.elapsed().as_secs_f64() / 60.0,
        verdict: String::from("measured in full"),
    });
    assert!(
        donor_rungs > 0,
        "the donor's measurement chose no rungs at all"
    );

    // --- everybody else borrows, and is checked ---------------------------------------
    for film in &films[1..] {
        let began = Instant::now();
        // Which tasks existed before, so the check's own can be told from the donor's.
        let before = task_ids(&state);
        let outcome = quality::quality_measure_reuse(&state, &donor_key, request(film)).await;
        // **The loan comes back at once and the check follows** (T478). The reading has to
        // wait for the verdict, and waiting for it here is also the proof that the loan is
        // provisional in the meantime: what the store says between these two lines is what a
        // build would see.
        let verdict = match &outcome {
            Ok(view) => {
                assert!(
                    view.run.check_pending,
                    "the loan came back already vouched for: nothing would stop a build \
                     starting before the check landed"
                );
                wait_for_the_check(&state, &view.run.source_key, &before).await
            }
            Err(_) => None,
        };
        let minutes = began.elapsed().as_secs_f64() / 60.0;
        match outcome {
            Ok(view) => {
                // What it has *now*, not what it had at the moment of the loan: a refused
                // check takes the rungs back, and printing the borrowed count beside the word
                // "refused" would say the opposite of what happened.
                let rungs = quality::quality_measure_result(&state, &view.run.source_key, "h264")
                    .await
                    .ok()
                    .and_then(|now| now.selection.map(|s| s.rungs.len()))
                    .unwrap_or(0);
                // What the check said, in its own words rather than in ours.
                let said = verdict
                    .unwrap_or_default()
                    .iter()
                    .filter(|d| format!("{:?}", d.key).contains("CheckPointHeld"))
                    .map(|d| {
                        let n =
                            |k: &str| d.params.get(k).and_then(|v| v.as_u64()).unwrap_or_default();
                        format!(
                            "held: {}.{:02} apart at {} Mbit/s @{}p",
                            n("apart") / 100,
                            n("apart") % 100,
                            n("bitrate"),
                            n("height")
                        )
                    })
                    .next()
                    .unwrap_or_else(|| {
                        // The task is where the refusal lives now, so read it from there.
                        the_new_task(&state, &before)
                            .map(|t| match t.error {
                                Some(e) => format!("refused: {e:?}"),
                                None => String::from("held, and said nothing"),
                            })
                            .unwrap_or_else(|| String::from("no check task at all"))
                    });
                table.push(Line {
                    film: name_of(film),
                    rungs,
                    minutes,
                    verdict: said,
                });
            }
            Err(e) => table.push(Line {
                film: name_of(film),
                rungs: 0,
                minutes,
                verdict: format!("refused: {e:?}"),
            }),
        }
    }

    // --- the reading ------------------------------------------------------------------
    println!("\n=== T447: a season through the application ===");
    println!(
        "{:<32} {:>5} {:>8}  what happened",
        "film", "rungs", "minutes"
    );
    let mut total = 0.0;
    for line in &table {
        println!(
            "{:<32} {:>5} {:>8.1}  {}",
            line.film, line.rungs, line.minutes, line.verdict
        );
        total += line.minutes;
    }
    let borrowed: f64 = table[1..].iter().map(|l| l.minutes).sum();
    println!(
        "\n{} films, {:.1} minutes altogether; the donor alone took {:.1}",
        table.len(),
        total,
        table[0].minutes
    );
    println!(
        "borrowing and checking cost {:.1} minutes for {} films — {:.0}% of one full measurement each",
        borrowed,
        table.len() - 1,
        100.0 * (borrowed / (table.len() - 1) as f64) / table[0].minutes
    );
}

fn name_of(path: &str) -> String {
    std::path::Path::new(path)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_owned())
        .chars()
        .take(32)
        .collect()
}

/// The newest check task, whatever became of it.
/// Every task there is, by number.
fn task_ids(state: &AppState) -> Vec<String> {
    vrcast_studio_lib::commands::api::tasks_list(state)
        .map(|all| all.into_iter().map(|t| t.id).collect())
        .unwrap_or_default()
}

/// The task the loan just started — the one that was not there a moment ago.
///
/// **By difference rather than by "the newest"**, which is what this did at first and why it
/// hung: whichever end of the list `.pop()` took, it was as likely to hand back the donor's
/// own completed measurement as the check, and a completed task never fails, so the wait
/// went round until its deadline.
fn the_new_task(
    state: &AppState,
    before: &[String],
) -> Option<vrcast_studio_lib::tasks::store::TaskRecord> {
    vrcast_studio_lib::commands::api::tasks_list(state)
        .ok()?
        .into_iter()
        .find(|t| !before.contains(&t.id))
}

/// Wait for the loan's check to land, and give back what it said.
///
/// `None` when it refused: the notice lives on the task only when it held.
async fn wait_for_the_check(
    state: &AppState,
    source_key: &str,
    before: &[String],
) -> Option<Vec<vrcast_studio_lib::domain::wording::Detail>> {
    use vrcast_studio_lib::tasks::state::TaskState;
    let deadline = Instant::now() + Duration::from_secs(600);
    while Instant::now() < deadline {
        // The mark clearing is the verdict landing; the task carries what it said.
        let still = vrcast_studio_lib::commands::quality::api::quality_measurements(state)
            .await
            .ok()
            .and_then(|all| all.into_iter().find(|r| r.source_key == source_key));
        match still {
            // Taken back: the check refused it.
            None => return None,
            Some(run) if !run.check_pending => {
                return the_new_task(state, before).map(|t| t.notices);
            }
            _ => {}
        }
        if let Some(task) = the_new_task(state, before) {
            if matches!(task.state, TaskState::Failed | TaskState::Cancelled) {
                return None;
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    None
}
