//! T277 — shutting the door on password guessing.
//!
//! **This is not only about security, and that is the part worth knowing.** The guessing is
//! constant, and with `maxstartups 10:30:100` the flood fills the slots for unauthenticated
//! connections and cuts long working sessions — that is how half a built quality set was lost
//! on the live server. Turning it away keeps the machine usable, not merely safe.

use futures::future::BoxFuture;

use crate::domain::deploy_steps::{Change, Checked, StepId};

use super::{Context, DeployError, Result, Step};

const JAIL: &str = "/etc/fail2ban/jail.local";

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
            names: vec![String::from("fail2ban")],
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
        ctx.ran("DEBIAN_FRONTEND=noninteractive apt-get install -y -qq fail2ban")
            .await?;
        ctx.put_file(JAIL, JAIL_LOCAL).await?;
        let said = ctx
            .ran("systemctl enable --now fail2ban >/dev/null 2>&1 && echo done")
            .await?;
        if !said.contains("done") {
            return Err(DeployError::Step {
                id: StepId::Fail2ban,
                detail: String::from("fail2ban would not start"),
                advice: None,
            });
        }
        Ok(())
    })
}
