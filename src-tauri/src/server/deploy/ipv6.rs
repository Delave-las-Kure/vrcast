//! T276 — carrying out the person's choice about IPv6 (FR-135, FR-136).
//!
//! Two paths, and neither is a default. Keeping it means the protection covers it as fully as
//! IPv4 — which the firewall step does — so there is nothing more to do here. Turning it off
//! means the serving must not answer over it at all, **and that is checked by asking**, not by
//! reading back the setting we just wrote (R-20).
//!
//! The choice is put to the person rather than decided for them because it changes what their
//! domain records have to say and whether viewers on IPv6 can watch at all. A default here
//! would be a quiet decision about other people's viewers.

use futures::future::BoxFuture;

use crate::domain::deploy_steps::{Change, Checked, StepId};
use crate::domain::dns_verdict::Ipv6Choice;

use super::{Context, DeployError, Result, Step};

const SYSCTL: &str = "/etc/sysctl.d/99-vrcast-ipv6.conf";

const OFF: &str = "\
# Written by VRCast Studio. IPv6 is off by the owner's choice made at deployment.
net.ipv6.conf.all.disable_ipv6 = 1
net.ipv6.conf.default.disable_ipv6 = 1
net.ipv6.conf.lo.disable_ipv6 = 1
";

pub fn step<'a>() -> Step<Context<'a>> {
    Step {
        id: StepId::Ipv6,
        changes,
        check,
        apply,
    }
}

fn changes(ctx: &Context<'_>) -> Vec<Change> {
    match ctx.ipv6 {
        Ipv6Choice::Disable => vec![Change::TurnsIpv6Off],
        // Nothing of its own: keeping it means the firewall step covers it, and a plan that
        // listed a change here would be promising something that does not happen.
        Ipv6Choice::Keep => Vec::new(),
    }
}

fn check<'x, 'a>(ctx: &'x Context<'a>) -> BoxFuture<'x, Result<Checked>> {
    Box::pin(async move {
        if ctx.ipv6 == Ipv6Choice::Keep {
            return Ok(Checked::NotNeeded);
        }
        if ctx.machine.is_container() {
            // The kernel's IPv6 switches are not per-namespace in the way this needs, and a
            // container that reported them applied would be reporting the host's.
            return Ok(ctx.not_here("turning IPv6 off"));
        }
        // Asked of the machine's own addresses rather than of the file we wrote: a setting
        // written and never applied looks identical on disk to one in force.
        let off = ctx
            .asks(
                "[ \"$(sysctl -n net.ipv6.conf.all.disable_ipv6 2>/dev/null)\" = 1 ] \\
                 && [ -z \"$(ip -6 addr show scope global 2>/dev/null)\" ] \\
                 && echo yes || echo no",
            )
            .await?;
        Ok(if off {
            Checked::Applied
        } else {
            Checked::NotApplied
        })
    })
}

fn apply<'x, 'a>(ctx: &'x Context<'a>) -> BoxFuture<'x, Result<()>> {
    Box::pin(async move {
        if ctx.ipv6 == Ipv6Choice::Keep {
            return Ok(());
        }
        ctx.put_file(SYSCTL, OFF).await?;
        let said = ctx
            .ran(&format!("sysctl -p {SYSCTL} >/dev/null 2>&1 && echo done"))
            .await?;
        if !said.contains("done") {
            return Err(DeployError::Step {
                id: StepId::Ipv6,
                detail: String::from("the IPv6 settings would not take"),
                advice: None,
            });
        }
        Ok(())
    })
}
