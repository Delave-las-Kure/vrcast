//! T278 — security patches arrive on their own (FR-126).
//!
//! Usually already there on Ubuntu, and the step has to **see** that rather than reinstall it:
//! a deployment that reinstalls what is present is slow on every repeat and, worse, teaches
//! nobody anything about the difference between "done" and "done again".
//!
//! This is also what pays for taking Caddy from a repository instead of a pinned archive
//! (T252): the repository is covered by these updates, and an archive would be cut off from
//! them.

use futures::future::BoxFuture;

use crate::domain::deploy_steps::{Change, Checked, StepId};

use super::{Context, DeployError, Result, Step};

/// What is installed. Public for the same reason as in `fail2ban`: the inventory compares
/// against the step, not against a copy of the name.
pub const PACKAGE: &str = "unattended-upgrades";

pub fn step<'a>() -> Step<Context<'a>> {
    Step {
        id: StepId::UnattendedUpgrades,
        changes,
        check,
        apply,
    }
}

fn changes(_: &Context<'_>) -> Vec<Change> {
    vec![Change::InstallsPackages {
        names: vec![String::from(PACKAGE)],
    }]
}

fn check<'x, 'a>(ctx: &'x Context<'a>) -> BoxFuture<'x, Result<Checked>> {
    Box::pin(async move {
        // Installed and switched on. The package being present says nothing: its timer can be
        // masked, and then it sits there looking like protection.
        let running = ctx
            .asks(
                "dpkg-query -W -f='${Status}' unattended-upgrades 2>/dev/null | grep -q 'ok installed' \\
                 && systemctl is-enabled unattended-upgrades >/dev/null 2>&1 \\
                 && echo yes || echo no",
            )
            .await?;
        Ok(if running {
            Checked::Applied
        } else {
            Checked::NotApplied
        })
    })
}

fn apply<'x, 'a>(ctx: &'x Context<'a>) -> BoxFuture<'x, Result<()>> {
    Box::pin(async move {
        let said = ctx
            .ran(
                "set -e
export DEBIAN_FRONTEND=noninteractive
dpkg-query -W -f='${Status}' unattended-upgrades 2>/dev/null | grep -q 'ok installed' \\
  || apt-get install -y -qq unattended-upgrades
systemctl enable --now unattended-upgrades >/dev/null 2>&1 || true
echo done",
            )
            .await?;
        if !said.contains("done") {
            return Err(DeployError::Step {
                id: StepId::UnattendedUpgrades,
                detail: said.trim().to_owned(),
                advice: None,
            });
        }
        Ok(())
    })
}
