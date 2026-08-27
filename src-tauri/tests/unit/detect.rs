//! T264 — reading what a server said about itself.
//!
//! The live checks (`tests/integration/detect_live.rs`) cover the three ordinary machines.
//! What they cannot cover is the awkward answer: a machine without `ss`, a state file that
//! ends without a newline, a process name with a bracket in it. Those are rare and each of
//! them, misread, produces a **confident wrong answer** rather than a failure.

use vrcast_studio_lib::domain::server_state::{judge, Kind};
use vrcast_studio_lib::server::detect::read;

fn said(caddyfile: &str, video_dir: &str, serving: &str, name: &str, state: &str) -> String {
    format!(
        "caddyfile={caddyfile}\nvideo_dir={video_dir}\nserving={serving}\nserver_name={name}\n\
         --vrcast-state--\n{state}\n--vrcast-state--\n"
    )
}

#[test]
fn a_bare_answer_is_read_as_a_bare_machine() {
    let facts = read(&said("no", "no", "0", "", ""));
    assert!(facts.state_file.is_none());
    assert!(!facts.caddyfile_present);
    assert_eq!(facts.web_server_running, None);
    assert_eq!(judge(&facts).kind, Kind::Clean);
}

#[test]
fn a_web_server_with_no_name_still_counts_as_a_web_server() {
    // `ss` names the process only for somebody allowed to see it. A machine where the name is
    // withheld is still a machine that is serving, and reporting "nothing is serving" because
    // we were not told what it is would be how a foreign server gets deployed over.
    let facts = read(&said("no", "no", "1", "", ""));
    assert!(
        facts.web_server_running.is_some(),
        "a serving machine was read as idle because the name was withheld"
    );
    assert_eq!(judge(&facts).kind, Kind::Foreign);
}

#[test]
fn a_machine_without_ss_is_not_reported_as_serving() {
    // The other direction, and it matters just as much: `ss` missing gives an empty answer,
    // and an empty answer read as "yes" would make every bare machine look like somebody
    // else's — the application would refuse to deploy anywhere.
    let facts = read(&said("no", "no", "0", "", ""));
    assert_eq!(facts.web_server_running, None);
    assert_eq!(judge(&facts).kind, Kind::Clean);
}

#[test]
fn a_state_file_with_braces_on_their_own_lines_survives_the_fence() {
    // The reason the file is fenced rather than put on a line of its own: it is JSON, it has
    // newlines in it, and it ends with a `}` alone on a line. A parser that stopped at the
    // first closing brace would read half a file and call it broken — turning our own server
    // into a foreign one.
    let state =
        "{\n  \"vrcast_server_version\": 1,\n  \"steps_applied\": [\n    \"user-dirs\"\n  ]\n}";
    let facts = read(&said("yes", "yes", "1", "caddy", state));
    assert!(
        matches!(facts.state_file, Some(Ok(_))),
        "the fenced file was not read whole"
    );
    let state = judge(&facts);
    assert_eq!(state.kind, Kind::Managed);
    assert_eq!(state.server_version, Some(1));
}

#[test]
fn a_half_written_state_file_makes_the_machine_foreign_rather_than_bare() {
    let facts = read(&said("yes", "yes", "1", "caddy", "{\"vrcast_server_ver"));
    assert!(matches!(facts.state_file, Some(Err(_))));
    assert_eq!(
        judge(&facts).kind,
        Kind::Foreign,
        "a marker we cannot read means we do not know what this machine is"
    );
}

#[test]
fn an_answer_with_lines_we_do_not_know_is_read_all_the_same() {
    // A shell that prints a banner, a warning from `ss`, a line from a profile — none of that
    // is ours and none of it may stop the reading. An answer that fails on an unexpected line
    // would fail on somebody's login banner and blame their server.
    let mut text = String::from("Welcome to Ubuntu 24.04 LTS\n");
    text.push_str(&said("no", "no", "0", "", ""));
    text.push_str("Last login: never\n");
    let facts = read(&text);
    assert_eq!(judge(&facts).kind, Kind::Clean);
}
