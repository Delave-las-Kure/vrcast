//! T393, T394 — there is somewhere to minimise to, or the close button closes.
//!
//! Checked against a stand-in loader, because the real one needs a Linux desktop session and
//! the rule it implements does not. What is checked is the rule: which libraries are tried,
//! in what order, and what the close button does with the answer.

use std::cell::RefCell;

use vrcast_studio_lib::tray::{close_action, probe_appindicator, CloseAction, TrayState};

/// A loader that answers for a fixed set of names and records what it was asked.
fn loader_with(
    present: &[&str],
) -> (
    impl Fn(&str) -> bool + 'static,
    &'static RefCell<Vec<String>>,
) {
    // A leak, on purpose and once per test: the recorder has to outlive the closure, and a
    // test binary that ends in a moment is the one place where that costs nothing.
    let asked: &'static RefCell<Vec<String>> = Box::leak(Box::new(RefCell::new(Vec::new())));
    let present: Vec<String> = present.iter().map(|s| (*s).to_owned()).collect();
    let f = move |name: &str| {
        asked.borrow_mut().push(name.to_owned());
        present.iter().any(|p| p == name)
    };
    (f, asked)
}

#[test]
fn the_maintained_library_is_found() {
    let (loads, _) = loader_with(&["libayatana-appindicator3.so.1"]);
    assert_eq!(probe_appindicator(loads), TrayState::Installed);
}

#[test]
fn the_older_library_is_found_too() {
    // **The one that would be quietly missed.** Ayatana is what current releases ship, so a
    // probe written on a current machine works everywhere its author looked — and reports
    // "no tray" on the long-term releases that still carry the older name, where a tray is
    // sitting right there.
    let (loads, _) = loader_with(&["libappindicator3.so.1"]);
    assert_eq!(probe_appindicator(loads), TrayState::Installed);
}

#[test]
fn nothing_present_means_nowhere_to_minimise_to() {
    let (loads, _) = loader_with(&[]);
    assert_eq!(probe_appindicator(loads), TrayState::Unavailable);
}

#[test]
fn every_name_is_tried_before_giving_up() {
    // Without this the check above passes just as well with half the list, and half the list
    // is exactly the fault: a name dropped here becomes a desktop where the window vanishes
    // into a tray that is not there.
    let (loads, asked) = loader_with(&[]);
    let _ = probe_appindicator(loads);
    let tried = asked.borrow().clone();
    assert_eq!(
        tried,
        vec![
            "libayatana-appindicator3.so.1".to_owned(),
            "libappindicator3.so.1".to_owned()
        ],
        "not every library was tried, or they were tried in the wrong order"
    );
}

#[test]
fn the_search_stops_at_the_first_one_that_loads() {
    // Loading a library is not free, and the second name is only ever an older spelling of
    // the first. Asking for it after an answer is already in hand is work for nothing.
    let (loads, asked) = loader_with(&["libayatana-appindicator3.so.1"]);
    let _ = probe_appindicator(loads);
    assert_eq!(
        asked.borrow().len(),
        1,
        "the search carried on after finding one"
    );
}

#[test]
fn the_close_button_hides_only_where_there_is_somewhere_to_hide() {
    assert_eq!(close_action(TrayState::Installed), CloseAction::Hide);
    assert_eq!(close_action(TrayState::Unavailable), CloseAction::Exit);
}

#[test]
fn a_machine_that_can_show_a_tray_says_so() {
    // The real probe, on whatever this is running on. Not an assertion about the answer —
    // it differs by machine, and a check that demanded one would be red on half of them —
    // but that asking does not panic and does not hang, which is what a hand-declared
    // `dlopen` most plausibly gets wrong.
    let _ = vrcast_studio_lib::tray::probe();
}

// ---------- the feature has to be on, or none of the above runs (T396) ----------

#[test]
fn the_tray_code_is_matched_by_the_feature_that_makes_it_work() {
    // **The one failure mode a stub loader cannot catch.** Everything above is pure Rust and
    // passes whether or not Tauri was built with `tray-icon`. Without the feature there is
    // no `TrayIconBuilder` to call, so the icon is never created — and the close button,
    // told by a probe that a tray exists, hides the window into nothing.
    //
    // Checked from the manifest rather than by calling anything: the alternative is a
    // compile error at the far end of a release build, and this is a second.
    use std::path::Path;

    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    if !root.join("src/tray").is_dir() {
        return; // No tray code, nothing to require.
    }

    let manifest =
        std::fs::read_to_string(root.join("Cargo.toml")).expect("Cargo.toml would not read");
    let line = manifest
        .lines()
        .find(|l| l.trim_start().starts_with("tauri = "))
        .expect("Cargo.toml no longer declares tauri");

    assert!(
        line.contains("tray-icon"),
        "src/tray exists and `tauri` is declared without the `tray-icon` feature:\n  {line}\n\n\
         The probe would report a tray, the close button would hide the window, and there \
         would be no icon to bring it back with."
    );
}

#[test]
fn the_deb_says_it_needs_the_library_the_tray_is_drawn_by() {
    // The icon is drawn by an AppIndicator implementation, and a `.deb` that does not say so
    // installs cleanly onto a machine without one. Then the probe reports no tray, the close
    // button closes — correct, and a silently worse application than the one that was meant.
    //
    // **Written as an alternative on purpose.** `libayatana-appindicator3-1` is what Ubuntu
    // 24.04 ships and it *Provides* `libappindicator3-1`; the older package does not provide
    // the newer name. Depending on the new name alone would refuse to install on a system
    // carrying only the old one, where the tray works perfectly. Read off the archive on the
    // throwaway stand on 2026-08-28: 0.5.93-1build3 and 12.10.1+20.10.20200706.1-0ubuntu5.
    //
    // There is no rpm target in this project, so there is no rpm dependency to get wrong.
    use std::path::Path;

    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    if !root.join("src/tray").is_dir() {
        return;
    }

    let conf: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(root.join("tauri.conf.json")).unwrap())
            .expect("tauri.conf.json is not valid JSON");

    let depends = conf
        .pointer("/bundle/linux/deb/depends")
        .and_then(|d| d.as_array())
        .expect("there is no bundle.linux.deb.depends, so the .deb declares no dependencies");

    assert!(
        depends
            .iter()
            .filter_map(|d| d.as_str())
            .any(|d| d.contains("appindicator")),
        "the .deb does not ask for an AppIndicator library: {depends:?}"
    );

    // The targets this project actually builds. If rpm is ever added, its dependency has a
    // different name and this check must be taught it rather than pass by not looking.
    let targets = conf
        .pointer("/bundle/targets")
        .and_then(|t| t.as_array())
        .expect("bundle.targets is gone");
    assert!(
        !targets.iter().any(|t| t.as_str() == Some("rpm")),
        "an rpm target was added, and its AppIndicator package is named differently — \
         verify the name on a real Fedora or RHEL before writing it down"
    );
}

// ---------- leaving through the menu, and what it costs first (T400, FR-086) ----------

#[test]
fn leaving_with_nothing_at_stake_asks_nothing() {
    // A dialog that always appears and always has one right answer is one people learn to
    // dismiss unread — and then the one that mattered goes past with the rest. FR-086 asks
    // for a warning **during running tasks**, not for a toll gate.
    use vrcast_studio_lib::tray::{quit_action, QuitAction};
    assert_eq!(quit_action(0), QuitAction::Straight);
}

#[test]
fn leaving_with_anything_at_stake_asks() {
    use vrcast_studio_lib::tray::{quit_action, QuitAction};
    assert_eq!(quit_action(1), QuitAction::Ask);
    assert_eq!(quit_action(7), QuitAction::Ask);
}

#[test]
fn the_tray_menu_asks_before_it_ends_the_application() {
    // ⚠ **Read out of the source, because nothing else can read it.** The menu handler is a
    // closure handed to Tauri's builder; there is no way to call it without a desktop session
    // and a real icon, and on Linux even a click cannot be waited for (R-35). What the check
    // is for is one line and the whole of FR-086 on this path: until T400 it said
    // `QUIT => app.exit(0)`, and somebody with a thirty-gigabyte upload running chose "Exit"
    // and lost it without a word.
    //
    // Crude on purpose. The alternative was no check at all on the one line that decides
    // whether a person is warned.
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/tray/mod.rs");
    let text = std::fs::read_to_string(&path).expect("the tray module would not read");

    let at = text
        .find(".on_menu_event(")
        .expect("the tray menu no longer answers its items — has the handler been renamed?");
    let closing = text[at..]
        .find("\n            })")
        .expect("the menu handler's end was not found; this check has come adrift from the file");
    let handler = &text[at..at + closing];

    assert!(
        handler.contains("QUIT => quit_pressed(app)"),
        "the tray menu's \"Exit\" no longer goes through `quit_pressed`, which is where the \
         cost of leaving is counted:\n{handler}"
    );
    assert!(
        !handler.contains("exit("),
        "the tray menu ends the application from inside its own handler. That is what it did \
         before T400, and it meant leaving without being told what leaving costs (FR-086):\n\
         {handler}"
    );
}

#[test]
fn not_being_able_to_count_the_cost_is_treated_as_a_cost() {
    // The other half of `quit_pressed`, and the reason it is written the way it is: when the
    // core cannot be reached, `at_stake` is 1, not 0. Not knowing is not the same as knowing
    // there is nothing, and exiting on a question nobody could answer is the one outcome that
    // cannot be taken back.
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/tray/mod.rs");
    let text = std::fs::read_to_string(&path).expect("the tray module would not read");
    let at = text
        .find("fn quit_pressed")
        .expect("`quit_pressed` is gone — the menu decides some other way now");
    let body = &text[at..(at + 1200).min(text.len())];

    assert!(
        body.contains("unwrap_or(1)") && body.contains("None => 1"),
        "a tray \"Exit\" that cannot reach the core now falls through to leaving. Both ways of \
         failing to count must read as \"something is at stake\":\n{body}"
    );
}
