//! T271 — the serving's configuration.
//!
//! The reference travels with the application (`resources/server/`, T250) and is checked
//! against the skill's copy on every local run (T251), because the skill stays the working
//! fallback: when the application will not start, or is being distrusted, the skill is what
//! the serving is brought back with. Two references that have quietly drifted apart mean the
//! fallback repairs a server the application does not recognise.
//!
//! **Validated before the service is asked to use it.** A configuration checked afterwards is
//! one whose mistake is discovered by the serving falling over — and on a first deployment
//! that is the only thing the person has.

use futures::future::BoxFuture;

use crate::domain::deploy_steps::{Change, Checked, StepId};

use super::{Context, DeployError, Result, Step};

/// The main configuration. Created here and, from then on, **read only** — the ownership rule
/// of `contracts/server-contract.md`. If a person edits it by hand, the difference is noticed
/// and shown, and the file is not overwritten.
const CADDYFILE: &str = "/etc/caddy/Caddyfile";

/// The rules file the application owns outright (R-03). Everything to do with capping a
/// viewer's quality is written here and nowhere else, so a mistake in a rule costs a rule
/// rather than the whole of the serving.
const LIMITS: &str = "/etc/caddy/vrcast-limits.conf";

/// The reference, carried in the binary. `include_str!` and not a file read at run time:
/// a deployment must not depend on a file sitting next to the application on the person's
/// machine, which is a thing that goes missing.
const REFERENCE: &str = include_str!("../../../resources/server/Caddyfile");

/// What the empty rules file holds.
///
/// A line rather than nothing: importing a file that matches nothing is an error and Caddy
/// would refuse to start, and an entirely empty one draws a warning on every validate — and a
/// warning that is always there teaches people not to read warnings.
const LIMITS_EMPTY: &str = "\
# The quality-limit rules. This file belongs to VRCast Studio: it is rewritten whole
# on every change, and anything added here by hand will be lost.
";

/// The reference with this server's domain in it.
pub fn caddyfile_for(domain: &str) -> String {
    REFERENCE.replace("{$SERVER_DOMAIN}", domain)
}

pub fn step<'a>() -> Step<Context<'a>> {
    Step {
        id: StepId::Configs,
        changes,
        check,
        apply,
    }
}

fn changes(_: &Context<'_>) -> Vec<Change> {
    vec![
        Change::WritesFile {
            path: String::from(CADDYFILE),
        },
        Change::WritesFile {
            path: String::from(LIMITS),
        },
    ]
}

fn check<'x, 'a>(ctx: &'x Context<'a>) -> BoxFuture<'x, Result<Checked>> {
    Box::pin(async move {
        let wanted = caddyfile_for(ctx.domain);
        if !ctx.file_is(CADDYFILE, &wanted).await? {
            return Ok(Checked::NotApplied);
        }
        // The rules file only has to exist: after the first deployment its contents belong to
        // the quality limits, and demanding the empty version back would wipe every cap the
        // moment anybody re-ran the deployment.
        let there = ctx
            .asks(&format!("test -f {LIMITS} && echo yes || echo no"))
            .await?;
        Ok(if there {
            Checked::Applied
        } else {
            Checked::NotApplied
        })
    })
}

fn apply<'x, 'a>(ctx: &'x Context<'a>) -> BoxFuture<'x, Result<()>> {
    Box::pin(async move {
        // The rules file first: the main configuration imports it, and a configuration
        // importing a file that is not there yet does not validate.
        let there = ctx
            .asks(&format!("test -f {LIMITS} && echo yes || echo no"))
            .await?;
        if !there {
            ctx.put_file(LIMITS, LIMITS_EMPTY).await?;
        }

        ctx.put_file(CADDYFILE, &caddyfile_for(ctx.domain)).await?;

        let said = ctx
            .ran(&format!(
                "caddy validate --adapter caddyfile --config {CADDYFILE} 2>&1 | tail -n 3; echo rc=$?"
            ))
            .await?;
        if !said.contains("Valid configuration") {
            return Err(DeployError::Step {
                id: StepId::Configs,
                detail: said.trim().to_owned(),
                advice: None,
            });
        }

        // **The step lays its own trap and has to clear it.** `caddy validate` does not
        // merely parse: it brings the servers up for a moment, and that **creates**
        // /var/log/caddy/access.log — owned by root, because that is who we are. The
        // service afterwards runs as `caddy`, cannot open the file, and dies with
        // "permission denied" at the services step, which has nothing to do with logging.
        //
        // Recorded in the skill as a trap to watch for; found here as a trap we set
        // ourselves, by the mechanism asking again after applying (2026-08-27).
        ctx.ran("chown -R caddy:caddy /var/log/caddy 2>/dev/null || true")
            .await?;
        Ok(())
    })
}
