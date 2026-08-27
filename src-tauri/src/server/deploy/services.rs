//! T272 — starting the serving, and knowing that it is serving **ours**.
//!
//! **Not "the command returned zero".** `systemctl enable` writes a symlink and says nothing
//! about whether anything runs; `systemctl start` returns before a service has necessarily
//! settled.
//!
//! **And not "the service is running" either** — found on the real stand (2026-08-27). The
//! Caddy package starts the service the moment it is installed, with the distribution's own
//! configuration: a bare site on port 80. Our configuration is written a step later, and if
//! the reload after it does not take, the service goes on running — happily, enabled and
//! active — serving somebody else's file. Every check passed and the domain answered
//! nothing, because there was no certificate and nothing on 443.
//!
//! So the question is not whether it is up but whether the configuration it is holding is
//! ours, and that is asked of Caddy itself through its own administrative endpoint.

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
        let domain = crate::server::shell_quote(ctx.domain);
        // Three halves, and the third is the one that was missing. Enabled and not running
        // means the serving is down until somebody reboots; running and not enabled means
        // it is up until somebody does; and running **the wrong configuration** means it is
        // up, answers nothing anybody wants, and passes both of the other two.
        let up = ctx
            .asks(&format!(
                "[ \"$(systemctl is-enabled {SERVICE} 2>/dev/null)\" = enabled ] \\
                 && [ \"$(systemctl is-active {SERVICE} 2>/dev/null)\" = active ] \\
                 && curl -sf --max-time 5 http://127.0.0.1:2019/config/ 2>/dev/null | grep -qF {domain} \\
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
# A reload rather than a restart: a restart drops every viewer's connection, and a
# deployment repeated on a working server has no business doing that.
#
# **Its failure is NOT swallowed.** It was, and that is how a service left running the
# distribution's own configuration passed for a working deployment. A restart is the
# fallback — on a server that is not yet serving anybody there is nothing to drop, and on
# one that is, the reload will have worked.
systemctl reload {SERVICE} || systemctl restart {SERVICE}
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
