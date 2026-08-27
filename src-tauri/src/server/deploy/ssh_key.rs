//! T273 — our key on the server, **proved by logging in with it**.
//!
//! Before anything touches the password (R-12). The other way round the application turns off
//! the way it got in before it has another, and getting back means the hosting provider's
//! console — which the person may not have, may not remember the password for, and will be
//! looking for at the worst moment.
//!
//! The proof is a **new connection**. The one we are already using stays open whatever we do
//! to the settings, which is exactly what makes it the wrong witness: it would say the key
//! works when what it really says is that we were already in.

use futures::future::BoxFuture;

use crate::domain::deploy_steps::{Change, Checked, StepId};

use super::{Context, DeployError, Result, Step};

pub fn step<'a>() -> Step<Context<'a>> {
    Step {
        id: StepId::SshKey,
        changes,
        check,
        apply,
    }
}

fn changes(_: &Context<'_>) -> Vec<Change> {
    vec![Change::AddsSshKey]
}

fn check<'x, 'a>(ctx: &'x Context<'a>) -> BoxFuture<'x, Result<Checked>> {
    Box::pin(async move {
        // The file first, because it is cheap, and only then a connection. Asking for a
        // connection on every check of every run would be a login attempt per step.
        let line = crate::server::shell_quote(ctx.public_key.trim());
        let in_file = ctx
            .asks(&format!(
                "grep -qxF {line} /root/.ssh/authorized_keys 2>/dev/null && echo yes || echo no"
            ))
            .await?;
        if !in_file {
            return Ok(Checked::NotApplied);
        }
        Ok(if (ctx.proofs.key_works)().await {
            Checked::Applied
        } else {
            // In the file and not working: the wrong permissions on the directory, or a
            // server configured to ignore the file. Reported as not applied so the step runs
            // and fixes what it can, rather than as done — which is the answer that leaves a
            // person locked out one step later.
            Checked::NotApplied
        })
    })
}

fn apply<'x, 'a>(ctx: &'x Context<'a>) -> BoxFuture<'x, Result<()>> {
    Box::pin(async move {
        let line = crate::server::shell_quote(ctx.public_key.trim());
        let said = ctx
            .ran(&format!(
                "set -e
mkdir -p /root/.ssh
chmod 700 /root/.ssh
touch /root/.ssh/authorized_keys
# Appended if it is not there, never rewritten whole: the file may hold keys the person put
# there themselves, and a deployment that quietly removed somebody's own way in would be the
# worst kind of helpful.
grep -qxF {line} /root/.ssh/authorized_keys || printf '%s\\n' {line} >> /root/.ssh/authorized_keys
chmod 600 /root/.ssh/authorized_keys
echo done"
            ))
            .await?;

        if !said.contains("done") {
            return Err(DeployError::Step {
                id: StepId::SshKey,
                detail: said.trim().to_owned(),
                advice: None,
            });
        }

        // **Proved here rather than left to the re-check.** The next step turns the password
        // off; if the key does not in fact work, that is the moment the door closes, and it
        // has to be refused before then and not after.
        if !(ctx.proofs.key_works)().await {
            return Err(DeployError::Step {
                id: StepId::SshKey,
                detail: String::from(
                    "the key is in authorized_keys and a fresh login with it does not work",
                ),
                advice: None,
            });
        }
        Ok(())
    })
}
