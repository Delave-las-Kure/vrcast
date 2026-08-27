//! T270 — the user the serving runs as, and the directories it serves from.
//!
//! **The log's owner is a recorded trap.** A log file created by root — which is what happens
//! if anything is run as root before Caddy first starts — cannot be opened by Caddy under its
//! own user, and the reload fails with "permission denied" at a step that has nothing to do
//! with logging. That is why the ownership is part of the check and not only of the apply.

use futures::future::BoxFuture;

use crate::domain::deploy_steps::{Change, Checked, StepId};

use super::{Context, DeployError, Result, Step};

/// The system user the videos belong to.
pub const OWNER: &str = "vrcast";
/// Where the application's own things live, apart from the videos.
const HOME: &str = "/var/lib/vrcast";
const OPT: &str = "/opt/vrcast";
/// Caddy's log directory, owned by Caddy rather than by us.
const LOG_DIR: &str = "/var/log/caddy";
const LOG_OWNER: &str = "caddy";
/// Where the state file goes (T281). Made here so the last step has somewhere to write.
const STATE_DIR: &str = "/etc/vrcast";

pub fn step<'a>() -> Step<Context<'a>> {
    Step {
        id: StepId::UserDirs,
        changes,
        check,
        apply,
    }
}

fn changes(ctx: &Context<'_>) -> Vec<Change> {
    let mut changes = vec![Change::CreatesSystemUser {
        name: String::from(OWNER),
    }];
    for path in [ctx.video_dir, OPT, LOG_DIR, STATE_DIR] {
        changes.push(Change::CreatesDirectory {
            path: String::from(path),
        });
    }
    changes
}

fn check<'x, 'a>(ctx: &'x Context<'a>) -> BoxFuture<'x, Result<Checked>> {
    Box::pin(async move {
        let video_dir = crate::server::shell_quote(ctx.video_dir);
        let all_there = ctx
            .asks(&format!(
                "ok=1
id {OWNER} >/dev/null 2>&1 || ok=0
for d in {video_dir} {OPT} {LOG_DIR} {STATE_DIR}; do
  [ -d \"$d\" ] || ok=0
done
# The ownership, not merely the existence. A directory owned by the wrong user reads as done
# and then refuses every write, at a step that will look like the one at fault.
[ \"$(stat -c %U {video_dir} 2>/dev/null)\" = {OWNER} ] || ok=0
[ \"$(stat -c %U {LOG_DIR} 2>/dev/null)\" = {LOG_OWNER} ] || ok=0
# The log FILE too, when it exists. It is created by whoever first started a server —
# and `caddy validate`, run as root, is one of those. Owned by root it cannot be opened
# by the service, and the serving does not come up.
if [ -e {LOG_DIR}/access.log ]; then
  [ \"$(stat -c %U {LOG_DIR}/access.log)\" = {LOG_OWNER} ] || ok=0
fi
[ $ok -eq 1 ] && echo yes || echo no"
            ))
            .await?;
        Ok(if all_there {
            Checked::Applied
        } else {
            Checked::NotApplied
        })
    })
}

fn apply<'x, 'a>(ctx: &'x Context<'a>) -> BoxFuture<'x, Result<()>> {
    Box::pin(async move {
        let video_dir = crate::server::shell_quote(ctx.video_dir);
        let said = ctx
            .ran(&format!(
                "set -e
# A system user with no shell and no way to log in: it exists to own files and to run a
# service, and an account that can be logged into is one more door.
id {OWNER} >/dev/null 2>&1 || useradd --system --home {HOME} --shell /usr/sbin/nologin {OWNER}
mkdir -p {video_dir} {OPT} {LOG_DIR} {STATE_DIR}
chown -R {OWNER}:{OWNER} {HOME} {OPT}
chown -R {OWNER}:{OWNER} {video_dir}
# Caddy's, not ours. See the note at the top of this file.
chown {LOG_OWNER}:{LOG_OWNER} {LOG_DIR}
echo done"
            ))
            .await?;

        if !said.contains("done") {
            return Err(DeployError::Step {
                id: StepId::UserDirs,
                detail: said.trim().to_owned(),
                advice: None,
            });
        }
        Ok(())
    })
}
