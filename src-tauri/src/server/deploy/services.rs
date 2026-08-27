//! T272 — starting the serving, and knowing that it started.
//!
//! **Not "the command returned zero".** `systemctl enable` writes a symlink and says nothing
//! about whether anything runs; `systemctl start` returns before a service has necessarily
//! settled. What is asked is what the service manager says about it afterwards, which is the
//! same rule the whole mechanism turns on.

use futures::future::BoxFuture;

use crate::domain::deploy_steps::{Change, Checked, StepId};

use super::{Context, DeployError, Result, Step};

/// The one service the serving is. MediaMTX is deliberately absent (T252): the application
/// never went that way, and a service nobody uses is one more thing to keep alive and to
/// explain when it is not.
const SERVICE: &str = "caddy";

pub fn step<'a>() -> Step<Context<'a>> {
    Step {
        id: StepId::Services,
        changes,
        check,
        apply,
    }
}

fn changes(_: &Context<'_>) -> Vec<Change> {
    vec![Change::EnablesService {
        name: String::from(SERVICE),
    }]
}

fn check<'x, 'a>(ctx: &'x Context<'a>) -> BoxFuture<'x, Result<Checked>> {
    Box::pin(async move {
        // Both halves. Enabled and not running means the serving is down until somebody
        // reboots; running and not enabled means it is up until somebody does.
        let up = ctx
            .asks(&format!(
                "[ \"$(systemctl is-enabled {SERVICE} 2>/dev/null)\" = enabled ] \\
                 && [ \"$(systemctl is-active {SERVICE} 2>/dev/null)\" = active ] \\
                 && echo yes || echo no"
            ))
            .await?;
        Ok(if up {
            Checked::Applied
        } else {
            Checked::NotApplied
        })
    })
}

fn apply<'x, 'a>(ctx: &'x Context<'a>) -> BoxFuture<'x, Result<()>> {
    Box::pin(async move {
        let said = ctx
            .ran(&format!(
                "set -e
systemctl daemon-reload
systemctl enable --now {SERVICE}
# A reload rather than a restart when it was already up: a restart drops every viewer's
# connection, and a deployment being repeated on a working server has no business doing that.
systemctl reload {SERVICE} 2>/dev/null || true
echo done"
            ))
            .await?;

        if !said.contains("done") {
            // What the service manager itself says, rather than our own summary: the reason a
            // service will not start is in its journal, and a person reading "the services step
            // failed" has to go and find it by hand.
            let why = ctx
                .ran(&format!(
                    "systemctl status {SERVICE} --no-pager --lines=10 2>&1 | tail -n 12"
                ))
                .await
                .unwrap_or_default();
            return Err(DeployError::Step {
                id: StepId::Services,
                detail: format!("{}\n{}", said.trim(), why.trim()),
                advice: None,
            });
        }
        Ok(())
    })
}
