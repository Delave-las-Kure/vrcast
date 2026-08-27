//! T264 — looking at a server and saying what it is (FR-120).
//!
//! Gathers the facts; `domain::server_state` judges them. The split is not tidiness: the
//! judging is where the dangerous mistakes live — a foreign server reported as bare, and the
//! application deploying over somebody else's machine — and it has to be checkable without a
//! server to check it against.
//!
//! **Everything in one command.** The connection is one and its channels are eight, two of
//! them already held by the watching of viewers (R-04). Five little commands would cost five
//! channel slots in turn, and they would be taken at the exact moment a person opens a server
//! — which is also when the library listing and the viewer watch start.

use crate::domain::server_state::{self, Facts, ServerState};
use crate::ssh::{Connection, Result};

/// What is asked, and how the answer is laid out.
///
/// One block of shell, one answer, parsed by line. The state file is fenced rather than put
/// on a line of its own because it is JSON with newlines in it — and a fence that cannot
/// appear inside JSON is what keeps a file with a `}` on its own line from ending the block
/// early.
const FENCE: &str = "--vrcast-state--";

/// Public so the checks can send **this** command rather than one written out again beside
/// them. A copy in a test goes stale the first time this one is edited, and the check then
/// passes while reporting on a question the application no longer asks.
///
/// The serving directory comes from the person's profile rather than being written in
/// here. It has one settled value on a server this application deployed — the contract
/// with the server names it — but a server set up by hand, or an older one, or one whose
/// disk is mounted elsewhere keeps its videos where its owner put them, and FR-004 says
/// an address of any kind comes from the profile. Caught by `check-no-hardcoded-server.sh`,
/// which is exactly what that check is for.
pub fn command(video_dir: &str) -> String {
    // A plain string and two replacements, NOT `format!`. The macro fills `{FENCE}` in
    // from a constant of that name in scope, so the replacement below did nothing at all
    // — it happened to give the same text, and would have gone on doing so until the
    // fence changed. `{VIDEO_DIR}` is what showed it: there is no constant by that name,
    // and the macro stopped compiling.
    //
    // `2>/dev/null` on each question: a server where /etc/vrcast cannot be read by us must
    // come back as "no file", not as a command that failed and took the whole detection
    // with it.
    const ASK: &str = r#"
printf 'caddyfile=%s\n' "$(test -f /etc/caddy/Caddyfile && echo yes || echo no)"
printf 'video_dir=%s\n' "$(test -d {VIDEO_DIR} && echo yes || echo no)"
printf 'ours=%s\n' "$(test -e /etc/caddy/vrcast-limits.conf -o -d /var/lib/vrcast && echo yes || echo no)"
printf 'serving=%s\n' "$(ss -ltnH 2>/dev/null | awk '$4 ~ /:(80|443)$/' | head -n 1 | wc -l)"
printf 'server_name=%s\n' "$(ss -ltnpH 2>/dev/null | awk '$4 ~ /:(80|443)$/' | grep -o 'users:((\"[^\"]*\"' | head -n 1 | sed 's/.*((\"//;s/\"//')"
printf '%s\n' '{FENCE}'
cat /etc/vrcast/state.json 2>/dev/null
printf '\n%s\n' '{FENCE}'
"#;

    ASK.replace("{FENCE}", FENCE)
        .replace("{VIDEO_DIR}", &super::shell_quote(video_dir))
}

/// Ask the server about itself and say what it is.
pub async fn detect(conn: &Connection, video_dir: &str) -> Result<ServerState> {
    let said = conn.exec(&command(video_dir)).await?;
    Ok(server_state::judge(&read(&said.stdout)))
}

/// Turn the answer into facts.
///
/// Separate from the asking so the parsing is checkable without a server — including the
/// awkward cases, which is where it matters: a web server whose name has a space in it, a
/// state file that ends without a newline, a machine where `ss` is not installed.
pub fn read(said: &str) -> Facts {
    let mut facts = Facts::default();
    let mut in_state = false;
    let mut state = String::new();
    let mut name = String::new();
    let mut serving = false;

    for line in said.lines() {
        if line.trim() == FENCE {
            // The second fence closes the block. Anything after it is not the file.
            in_state = !in_state;
            continue;
        }
        if in_state {
            state.push_str(line);
            state.push('\n');
            continue;
        }
        if let Some(value) = line.strip_prefix("caddyfile=") {
            facts.caddyfile_present = value.trim() == "yes";
        } else if let Some(value) = line.strip_prefix("video_dir=") {
            facts.video_dir_present = value.trim() == "yes";
        } else if let Some(value) = line.strip_prefix("ours=") {
            // The rules file this application owns outright, or its own directory under
            // /var/lib. Nobody else creates either, and both appear long before the state
            // file — which is what makes an interrupted deployment tellable from a
            // stranger's machine (T332).
            facts.our_own_marks = value.trim() == "yes";
        } else if let Some(value) = line.strip_prefix("serving=") {
            serving = value.trim() == "1";
        } else if let Some(value) = line.strip_prefix("server_name=") {
            name = value.trim().to_owned();
        }
    }

    // Something is listening on a serving port. Its name is a courtesy, not a condition: `ss`
    // will not name the process unless it is run as somebody who may see it, and a web server
    // we cannot name is still a web server. Reporting "nothing is serving" because the name
    // was withheld is how a foreign machine gets deployed over.
    if serving {
        facts.web_server_running = Some(if name.is_empty() {
            String::from("unknown")
        } else {
            name
        });
    }

    let state = state.trim();
    if !state.is_empty() {
        facts.state_file = Some(server_state::parse_state_file(state));
    }

    facts
}
