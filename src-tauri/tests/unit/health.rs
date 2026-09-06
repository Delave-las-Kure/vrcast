//! T312 — the readings that are easy to get backwards.
//!
//! Three of them, and each is a way of being wrong that costs more than saying nothing:
//! calling a working firewall broken, calling a stopped serving fine, and calling a machine
//! that is reading its disk instead of its memory perfectly healthy.

use vrcast_studio_lib::domain::health::{
    self, Delivery, Disk, Memory, Rating, Reading, Service, Snapshot, Tuning,
};

fn ok() -> Snapshot {
    Snapshot {
        services: vec![Service {
            name: String::from("caddy"),
            state: String::from("active"),
        }],
        firewall_status: Some(String::from("active")),
        memory: Memory {
            total_mb: 1900,
            used_mb: 400,
            buff_cache_mb: 1200,
            swap_total_mb: 1024,
            swap_used_mb: 0,
        },
        disk: Disk {
            used_mb: 20_000,
            free_mb: 20_000,
        },
        tuning: Tuning {
            congestion: Some(String::from("bbr")),
            qdisc: Some(String::from("fq")),
            slow_start_after_idle: Some(false),
            readahead_kb: Some(8192),
            restart: Some(String::from("always")),
        },
        open_ports: vec![String::from("0.0.0.0:443"), String::from("0.0.0.0:22")],
        delivery: Delivery::Answered { status: 206 },
        watching_now: 0,
        container: false,
    }
}

fn rating_of(snap: &Snapshot, about: Reading) -> Rating {
    health::judge(snap)
        .into_iter()
        .find(|r| r.about == about)
        .unwrap_or_else(|| panic!("{about:?} was not judged at all"))
        .rating
}

#[test]
fn a_healthy_machine_says_nothing() {
    let judged = health::judge(&ok());
    assert_eq!(health::worst(&judged), Rating::Fine);
}

#[test]
fn a_firewall_that_exited_is_not_a_problem() {
    // The recorded trap. `is-active` answers `inactive` on a working firewall, because it is
    // a oneshot unit: it applies the rules and exits. A judgement reading that field calls a
    // healthy machine broken on every single snapshot, and then nobody reads the snapshots.
    let mut snap = ok();
    snap.services.push(Service {
        name: String::from("ufw"),
        state: String::from("inactive"),
    });
    assert_eq!(rating_of(&snap, Reading::Firewall), Rating::Fine);
    assert_eq!(health::worst(&health::judge(&snap)), Rating::Fine);

    // And a firewall that is genuinely off **is**.
    snap.firewall_status = Some(String::from("inactive"));
    assert_eq!(rating_of(&snap, Reading::Firewall), Rating::Trouble);
}

#[test]
fn a_stopped_serving_is_trouble_and_is_named() {
    let mut snap = ok();
    snap.services[0].state = String::from("failed");
    let judged = health::judge(&snap);
    let serving = judged
        .iter()
        .find(|r| r.about == Reading::Serving)
        .expect("the serving was not judged");

    assert_eq!(serving.rating, Rating::Trouble);
    // Named, not implied: T321 asks for the service by name, and on a machine running more
    // than one "something is down" leaves a person opening a console to find out which.
    assert_eq!(
        serving.say.params.get("service").and_then(|v| v.as_str()),
        Some("caddy")
    );
    assert_eq!(health::worst(&judged), Rating::Trouble);
}

#[test]
fn a_small_serving_cache_is_worth_a_look_only_while_somebody_is_watching() {
    let mut snap = ok();
    snap.memory.buff_cache_mb = 100;

    // Nobody watching: nothing was asked for, so nothing is cached. Not news.
    assert_eq!(rating_of(&snap, Reading::ServingCache), Rating::Fine);

    // Somebody watching, and the same 100 MB now means the disk is being read instead of
    // memory — which is the thing the readahead measurement was bought to avoid.
    snap.watching_now = 3;
    assert_eq!(rating_of(&snap, Reading::ServingCache), Rating::Watch);
    assert_eq!(health::worst(&health::judge(&snap)), Rating::Watch);
}

#[test]
fn serving_the_whole_file_instead_of_the_range_is_its_own_answer() {
    // A 200 to a range request is not a failure and not a success: the film plays and seeking
    // does not, and that complaint arrives as "it is broken" with nothing to do with the
    // network. Lumped in with either of the neighbouring answers it becomes unfindable.
    let mut snap = ok();
    snap.delivery = Delivery::Answered { status: 200 };
    assert_eq!(rating_of(&snap, Reading::Delivery), Rating::Watch);

    snap.delivery = Delivery::Answered { status: 502 };
    assert_eq!(rating_of(&snap, Reading::Delivery), Rating::Trouble);

    snap.delivery = Delivery::Silent;
    assert_eq!(rating_of(&snap, Reading::Delivery), Rating::Trouble);

    // And a server with no video on it yet is not broken for having nothing to serve.
    snap.delivery = Delivery::NothingToServe;
    assert_eq!(rating_of(&snap, Reading::Delivery), Rating::Fine);
}

#[test]
fn what_a_container_cannot_answer_is_not_answered_for_it() {
    let mut snap = ok();
    snap.container = true;
    snap.tuning = Tuning::default();
    assert_eq!(rating_of(&snap, Reading::Network), Rating::Unknown);
    assert_eq!(rating_of(&snap, Reading::Readahead), Rating::Unknown);
}

#[test]
fn what_could_not_be_established_never_hides_what_could() {
    // `Unknown` losing to `Fine` would be the whole point thrown away, but so would `Unknown`
    // beating `Trouble`: a badge showing "could not tell" over a stopped serving is a badge
    // that hides the one thing worth knowing.
    let mut snap = ok();
    snap.container = true;
    snap.tuning = Tuning::default();
    assert_eq!(health::worst(&health::judge(&snap)), Rating::Unknown);

    snap.services[0].state = String::from("inactive");
    assert_eq!(health::worst(&health::judge(&snap)), Rating::Trouble);
}

#[test]
fn a_disk_with_nothing_left_is_trouble_before_it_is_full() {
    let mut snap = ok();
    snap.disk = Disk {
        used_mb: 38_000,
        free_mb: 2_000,
    };
    assert_eq!(rating_of(&snap, Reading::DiskSpace), Rating::Trouble);

    snap.disk = Disk {
        used_mb: 34_000,
        free_mb: 6_000,
    };
    assert_eq!(rating_of(&snap, Reading::DiskSpace), Rating::Watch);
}

#[test]
fn the_measured_settings_are_read_against_the_measured_values() {
    let mut snap = ok();
    snap.tuning.congestion = Some(String::from("cubic"));
    assert_eq!(rating_of(&snap, Reading::Network), Rating::Watch);

    let mut snap = ok();
    snap.tuning.readahead_kb = Some(128);
    assert_eq!(rating_of(&snap, Reading::Readahead), Rating::Watch);

    // A serving that does not come back on its own lies there until somebody notices.
    let mut snap = ok();
    snap.tuning.restart = Some(String::from("no"));
    assert_eq!(rating_of(&snap, Reading::AutoRestart), Rating::Watch);
}

// ---------- a verdict that can only be "fine" (T527) ----------

/// ⚠ **A reading that can only answer "nothing to do" must say why it cannot say more.**
///
/// The module opens by promising it: "What is a choice is marked too, and marked plainly."
/// `ports` kept that promise — "listed, not judged: which ports ought to be open depends on
/// what else the owner runs" — and `memory` did not. It returned `Fine` for any amount of
/// memory in use whatever, with no comment at all, so a reading that had weighed nothing was
/// indistinguishable from one that had weighed something and been content. That is the
/// module's own opening warning about `Unknown` shown as `Fine`, one layer along.
///
/// **The check is on the source, and the property is exact**: a rating function whose body
/// never mentions `Watch` or `Trouble` cannot return either, so it must carry the mark. There
/// is no way to ask this of the behaviour — a function that always answers `Fine` and one
/// that answers `Fine` for every input a test happens to try are the same from outside.
///
/// It does not say memory *should* be judged. It says that not judging it is a decision, and
/// a decision has to be written down where the next person will meet it.
#[test]
fn a_reading_that_can_only_say_fine_says_why_it_cannot_say_more() {
    const MARK: &str = "Listed, not judged";

    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/domain/health.rs");
    let text = std::fs::read_to_string(&path).expect("could not read health.rs");

    let mut checked = 0usize;
    let mut silent = Vec::new();

    for (at, _) in text.match_indices("fn ") {
        // A rating function and nothing else: `fn name(...) -> Rated {`.
        let Some(line_end) = text[at..].find('\n') else {
            continue;
        };
        let head = &text[at..at + line_end];
        // A reading, and not one of the two constructors that also hand back a `Rated`:
        // a reading is the thing that looks at a snapshot and forms a verdict about it.
        if !head.ends_with("-> Rated {") || !head.contains("snap: &Snapshot") {
            continue;
        }
        let name = head[3..].split('(').next().unwrap_or_default().to_owned();

        // The body, by counting braces from the opening one of the signature.
        let open = at + head.rfind('{').expect("the signature ends with a brace");
        let mut depth = 0usize;
        let mut end = None;
        for (i, c) in text[open..].char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(open + i);
                        break;
                    }
                }
                _ => {}
            }
        }
        let body = &text[open..end.expect("a rating function is never closed")];
        checked += 1;

        let can_object = body.contains("Rating::Watch") || body.contains("Rating::Trouble");
        if !can_object && !body.contains(MARK) {
            silent.push(name);
        }
    }

    assert!(
        checked >= 8,
        "only {checked} rating functions were found — the reader has stopped matching how \
         they are written, and everything above would pass over nothing"
    );
    assert!(
        silent.is_empty(),
        "these readings can never answer anything but \"fine\", and do not say why: {silent:?}\n\
         Either give them a threshold, or mark the choice with \"{MARK}\" and the reason — \
         the module's own header promises that a choice is marked plainly."
    );
}
