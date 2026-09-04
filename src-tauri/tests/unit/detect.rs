//! T264 — reading what a server said about itself.
//!
//! The live checks (`tests/integration/detect_live.rs`) cover the three ordinary machines.
//! What they cannot cover is the awkward answer: a machine without `ss`, a state file that
//! ends without a newline, a process name with a bracket in it. Those are rare and each of
//! them, misread, produces a **confident wrong answer** rather than a failure.

use vrcast_studio_lib::domain::server_state::{judge, Kind};
use vrcast_studio_lib::server::detect::{command, read};

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

// ---------- what the probe may do on a machine that is not ours (T487) ----------
//
// ⚠ **This command runs before the refusal, not after it.** `gate::open` connects, calls
// `detect`, and only then decides whether the machine may be touched — so by the time a
// foreign server is refused, the text below has already run on it. Principle I is not
// "the application refuses to change somebody else's server"; it is that it does not change
// it. The refusal is checked in `gate.rs`. What runs before the refusal was checked by
// nothing at all until this.
//
// Today the probe reads and does nothing else — `test`, `ss`, `cat`, `printf` and filters.
// A `mkdir -p` added to it would run on somebody else's machine, and every one of this
// project's eight hundred checks would stay green.

/// Every word that may stand where a command stands in the probe, and why it is safe on a
/// machine that is not ours.
///
/// **An allow-list, not a list of forbidden verbs.** A list of `mkdir`, `touch`, `rm` is
/// complete only against the writes somebody thought of; the one that gets through is the one
/// nobody listed. This way an unfamiliar command is red until a person writes down why it
/// reads — which is the moment worth interrupting.
const READS_ONLY: &[(&str, &str)] = &[
    ("printf", "writes to standard output, to a format of ours"),
    ("echo", "the same, and only ever with a literal"),
    ("test", "asks about a path and answers"),
    ("cat", "reads a file to standard output"),
    ("ss", "lists sockets"),
    ("grep", "filters what it is handed"),
    (
        "awk",
        "filters; the way it can write is a redirection, caught separately",
    ),
    (
        "sed",
        "edits a stream; the way it can write a file is `-i`, caught separately",
    ),
    (
        "head",
        "passes the first lines of what it is handed on and drops the rest",
    ),
    ("wc", "counts what it is handed and prints the number"),
];

/// The command fragments of a shell text: each one a command word with its arguments.
///
/// **Not a shell parser, and it must not be taken for one.** It models the four things this
/// check rests on: a single-quoted stretch is inert, a backslash outside quotes makes the next
/// character literal, `$(` and a backtick each start a fresh command, and a command word
/// follows a separator.
/// Double quotes are deliberately *not* treated as inert — substitution happens inside them,
/// and treating `"$(mkdir /x)"` as a string is the one mistake that would let a write past.
///
/// Everything it does not model is why [`the_guard_catches_the_ordinary_ways_of_writing`]
/// exists: the parser is not trusted, it is made to bite on forgeries.
fn shell_fragments(text: &str) -> Vec<String> {
    let flat = text.replace("$(", "(");
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_single = false;
    let mut chars = flat.chars();

    while let Some(c) = chars.next() {
        if in_single {
            cur.push(c);
            if c == '\'' {
                in_single = false;
            }
            continue;
        }
        match c {
            // A backslash outside quotes carries the next character through whatever it is.
            // Without this the `'\''` a shell-quoted value is escaped with reads as a closing
            // quote followed by a live separator, and every path with an apostrophe in it
            // would be reported as an injection.
            '\\' => {
                cur.push(c);
                if let Some(next) = chars.next() {
                    cur.push(next);
                }
            }
            '\'' => {
                in_single = true;
                cur.push(c);
            }
            '\n' | ';' | '|' | '&' | '(' | ')' | '`' => {
                let piece = cur.trim().to_string();
                if !piece.is_empty() {
                    out.push(piece);
                }
                cur.clear();
            }
            _ => cur.push(c),
        }
    }
    let piece = cur.trim().to_string();
    if !piece.is_empty() {
        out.push(piece);
    }
    out
}

/// Everything in a shell text that could write, said in words. Empty means it only reads.
fn what_could_write(text: &str) -> Vec<String> {
    let mut found = Vec::new();

    for fragment in shell_fragments(text) {
        let Some(word) = fragment.split_whitespace().next() else {
            continue;
        };
        let word = word.trim_matches('"');
        if word.is_empty() {
            continue;
        }
        if !READS_ONLY.iter().any(|(name, _)| *name == word) {
            found.push(format!(
                "`{word}` stands where a command stands and is not among the ones that only \
                 read (in `{fragment}`)"
            ));
            continue;
        }
        // A stream editor told to edit the file instead of the stream.
        if matches!(word, "sed" | "perl")
            && fragment
                .split_whitespace()
                .any(|a| a == "-i" || a == "--in-place" || a.starts_with("-i"))
        {
            found.push(format!("`{word}` is asked to edit in place: `{fragment}`"));
        }
    }

    // A redirection writes whatever stood before it — including one inside an `awk` program,
    // which the fragments above never look into. `/dev/null` is the one target that is not a
    // write, and the probe uses it to keep a missing tool from taking the whole answer down.
    for (at, _) in text.match_indices('>') {
        let after = text[at + 1..].trim_start_matches('>').trim_start();
        if !after.starts_with("/dev/null") {
            let shown: String = text[at..].chars().take(28).collect();
            found.push(format!("a redirection that is not to /dev/null: `{shown}`"));
        }
    }

    found
}

#[test]
fn the_probe_that_runs_before_the_refusal_only_reads() {
    let text = command("/var/lib/vrcast/videos");
    let writes = what_could_write(&text);
    assert!(
        writes.is_empty(),
        "the probe runs on a server before the application knows whether it may touch it, \
         and it could write:\n  {}\n\nThe command was:\n{text}",
        writes.join("\n  ")
    );
}

#[test]
fn a_video_directory_cannot_smuggle_a_command_into_the_probe() {
    // The directory comes out of a profile somebody typed. Badly quoted, it is not a bug in
    // the probe but a command on somebody else's server — chosen by whoever last edited the
    // profile, which on a shared machine is not necessarily the person running this.
    for nasty in [
        "/videos'; mkdir -p /tmp/vrcast-was-here; echo '",
        "/videos$(touch /tmp/vrcast-was-here)",
        "/videos`touch /tmp/vrcast-was-here`",
        "/videos && rm -rf /",
        "/videos\n touch /tmp/vrcast-was-here",
    ] {
        let text = command(nasty);
        let writes = what_could_write(&text);
        assert!(
            writes.is_empty(),
            "a video directory of {nasty:?} put a command into the probe:\n  {}",
            writes.join("\n  ")
        );
    }
}

#[test]
fn the_guard_catches_the_ordinary_ways_of_writing() {
    // ⚠ **The guard above is worth exactly what this test proves.** It rests on a parser of
    // forty lines that is not a shell, and a parser that quietly fails to see a command is a
    // check that passes for the wrong reason — the failure this whole guard exists to stop.
    // So each ordinary way of writing is forged into the probe and must be caught.
    let honest = command("/var/lib/vrcast/videos");
    for (forgery, what) in [
        ("\nmkdir -p /var/lib/vrcast", "a directory made"),
        ("\ntouch /etc/vrcast/state.json", "a file touched"),
        ("\necho hello > /tmp/vrcast", "a redirection"),
        ("\necho hello >> /tmp/vrcast", "an appending redirection"),
        (
            "\nsed -i 's/a/b/' /etc/caddy/Caddyfile",
            "a file edited in place",
        ),
        ("\nsystemctl restart nginx", "a service restarted"),
        (
            "\ncat /etc/hostname | tee /tmp/vrcast",
            "a write through a pipe",
        ),
        (
            "\nprintf x | awk '{print > \"/tmp/vrcast\"}'",
            "a write from inside a filter's own program",
        ),
        (
            "printf 'x=%s\\n' \"$(mkdir -p /tmp/vrcast)\"",
            "a write inside double quotes, where substitution still happens",
        ),
        (
            "\ntest -d /x && mkdir -p /tmp/vrcast || echo no",
            "a write on the far side of a condition",
        ),
    ] {
        let forged = format!("{honest}{forgery}");
        assert!(
            !what_could_write(&forged).is_empty(),
            "{what} went unnoticed, so the guard on the probe is worth nothing:\n{forgery}"
        );
    }
}

#[test]
fn every_command_the_probe_is_allowed_carries_its_reason() {
    // A list entry with no reason is a command somebody waved through, and the next reader
    // has no way to tell it from one that was thought about.
    for (name, why) in READS_ONLY {
        assert!(
            why.len() > 10,
            "`{name}` is allowed in the probe with no reason written down"
        );
    }
}
