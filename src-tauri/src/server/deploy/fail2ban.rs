//! T277 — shutting the door on password guessing.
//!
//! **This is not only about security, and that is the part worth knowing.** The guessing is
//! constant, and with `maxstartups 10:30:100` the flood fills the slots for unauthenticated
//! connections and cuts long working sessions — that is how half a built quality set was lost
//! on the live server. Turning it away keeps the machine usable, not merely safe.

use futures::future::BoxFuture;

use crate::domain::deploy_steps::{Change, Checked, StepId};

use super::{Context, DeployError, Result, Step};

/// Что ставится. Открыто наружу, чтобы опись серверной части (T337) сверялась с этим, а не
/// с копией имени рядом с проверкой.
pub const PACKAGE: &str = "fail2ban";

const JAIL: &str = "/etc/fail2ban/jail.local";

/// How long the apply waits for the jail to actually start guarding, in seconds.
///
/// Thirty. On an idle machine it takes two or three; the figure is for a machine that is busy
/// doing everything else the deployment just asked of it.
const PATIENCE_S: u32 = 30;

/// Carried over from the skill, values and all.
///
/// `backend = systemd` because Ubuntu keeps sshd's log in the journal and not in a file — with
/// the default backend the jail watches a file that is never written and bans nobody, while
/// looking perfectly configured.
const JAIL_LOCAL: &str = "\
[DEFAULT]
backend = systemd
bantime  = 1h
findtime = 10m
maxretry = 4

[sshd]
enabled = true
";

pub fn step<'a>() -> Step<Context<'a>> {
    Step {
        id: StepId::Fail2ban,
        changes,
        check,
        apply,
    }
}

fn changes(_: &Context<'_>) -> Vec<Change> {
    vec![
        Change::InstallsPackages {
            names: vec![String::from(PACKAGE)],
        },
        Change::EnablesService {
            name: String::from("fail2ban"),
        },
    ]
}

fn check<'x, 'a>(ctx: &'x Context<'a>) -> BoxFuture<'x, Result<Checked>> {
    Box::pin(async move {
        // Running **and** watching sshd. A fail2ban that is up with no jail enabled is the
        // shape this fails in: everything looks installed and nothing is being watched.
        let guarding = ctx
            .asks(&format!(
                "[ \"$(systemctl is-active fail2ban 2>/dev/null)\" = active ] \\
                 && [ -f {JAIL} ] \\
                 && fail2ban-client status sshd >/dev/null 2>&1 \\
                 && echo yes || echo no"
            ))
            .await?;
        Ok(if guarding {
            Checked::Applied
        } else {
            Checked::NotApplied
        })
    })
}

fn apply<'x, 'a>(ctx: &'x Context<'a>) -> BoxFuture<'x, Result<()>> {
    Box::pin(async move {
        ctx.ran(&format!(
            "DEBIAN_FRONTEND=noninteractive apt-get install -y -qq {PACKAGE}"
        ))
        .await?;
        ctx.put_file(JAIL, JAIL_LOCAL).await?;
        // **Waiting for the jail, not for the unit.** `systemctl enable --now` comes back as
        // soon as the process is up, and fail2ban is then still reading its configuration and
        // starting jails — its socket does not exist yet, so asking about the jail right
        // afterwards answers no. The step was then declared not taken on a machine where it
        // had in fact worked, which is exactly the failure this project's re-check is meant to
        // catch and exactly the one that makes it useless when the wait is missing. Seen in CI
        // on 2026-08-27, on three deployments at once on one runner; never locally, because
        // locally nothing else was competing for the processor.
        //
        // The waiting happens **on the server**, in one command: thirty round trips over SSH
        // to poll would spend one of the eight channels for half a minute (R-04).
        let said = ctx
            .ran(&format!(
                "systemctl enable --now fail2ban >/dev/null 2>&1 || {{ echo 'fail2ban would not start'; exit 0; }}
for _ in $(seq 1 {PATIENCE_S}); do
  fail2ban-client status sshd >/dev/null 2>&1 && {{ echo done; exit 0; }}
  sleep 1
done
# Out of patience. What is said here is what a person will be shown, so it carries the
# machine's own words rather than ours.
echo \"the sshd jail is not guarding after {PATIENCE_S}s: unit=$(systemctl is-active fail2ban)\"
fail2ban-client status sshd 2>&1 | head -n 3"
            ))
            .await?;
        if !said.contains("done") {
            return Err(DeployError::Step {
                id: StepId::Fail2ban,
                detail: said.trim().to_owned(),
                advice: None,
            });
        }
        Ok(())
    })
}
