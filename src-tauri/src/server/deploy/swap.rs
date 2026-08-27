//! T268 — a swap file on a small machine (FR-134).
//!
//! **Before the packages, and that is not tidiness.** Installing is the peak of memory use in
//! the whole deployment; a swap file made after it is a swap file for next time. On the stand
//! — 961 MB and no swap, which is what the cheapest tier of VPS is, and what somebody buying
//! their first server buys — apt is killed part-way through without one.
//!
//! **In a container this cannot be settled at all** (T246, measured): `swapon` is refused
//! whatever the privileges, and worse, `free` inside reports the *host's* swap, so a check
//! that merely looked would pass on a machine that has none. The step says so rather than
//! guessing.

use futures::future::BoxFuture;

use crate::domain::deploy_steps::{Change, Checked, StepId};
use crate::domain::swap::{self, Swap};

use super::{Context, DeployError, Result, Step};

/// Where the file goes. Beside the root, where `df` measured the room for it.
const PATH: &str = "/swapfile";

pub fn step<'a>() -> Step<Context<'a>> {
    Step {
        id: StepId::Swap,
        changes,
        check,
        apply,
    }
}

fn wanted(ctx: &Context<'_>) -> Swap {
    swap::decide(
        ctx.machine.memory_mb,
        ctx.machine.swap_mb,
        ctx.machine.free_disk_mb,
    )
}

fn changes(ctx: &Context<'_>) -> Vec<Change> {
    match wanted(ctx) {
        Swap::Make { megabytes } => vec![Change::CreatesSwapFile { megabytes }],
        // Nothing will be done, so nothing is promised. A plan that listed a change it will
        // not make teaches people that the plan is approximate.
        Swap::NotNeeded | Swap::NoRoom { .. } => Vec::new(),
    }
}

fn check<'x, 'a>(ctx: &'x Context<'a>) -> BoxFuture<'x, Result<Checked>> {
    Box::pin(async move {
        if ctx.machine.is_container() {
            return Ok(ctx.not_here("swap"));
        }
        Ok(match wanted(ctx) {
            Swap::NotNeeded => Checked::NotNeeded,
            // No room is not "already done" and not "not needed". Left to the apply, which
            // fails and says how much was wanted against how much there is.
            Swap::NoRoom { .. } => Checked::NotApplied,
            Swap::Make { .. } => {
                // Asked of the kernel, not of a file on disk: a `/swapfile` that exists and
                // was never switched on is exactly the state a half-finished run leaves, and
                // reading its presence as success would leave the machine without swap.
                let on = ctx
                    .asks(&format!(
                        "swapon --show=NAME --noheadings 2>/dev/null | grep -qx {PATH} && echo yes || echo no"
                    ))
                    .await?;
                if on {
                    Checked::Applied
                } else {
                    Checked::NotApplied
                }
            }
        })
    })
}

fn apply<'x, 'a>(ctx: &'x Context<'a>) -> BoxFuture<'x, Result<()>> {
    Box::pin(async move {
        let megabytes = match wanted(ctx) {
            Swap::Make { megabytes } => megabytes,
            Swap::NotNeeded => return Ok(()),
            Swap::NoRoom { wanted_mb, free_mb } => {
                return Err(DeployError::Step {
                    id: StepId::Swap,
                    detail: format!(
                        "a swap file of {wanted_mb} MB is needed and only {free_mb} MB are free"
                    ),
                    advice: None,
                })
            }
        };

        // `fallocate` first because it is instant; `dd` as the fallback because fallocate
        // fails on some file systems and a swap file has to be a real one either way.
        //
        // The old file is switched off and removed before making a new one: a half-made file
        // left by an interrupted run would otherwise be handed to mkswap as it stands.
        let said = ctx
            .ran(&format!(
                "set -e
swapoff {PATH} 2>/dev/null || true
rm -f {PATH}
fallocate -l {megabytes}M {PATH} 2>/dev/null || dd if=/dev/zero of={PATH} bs=1M count={megabytes} status=none
chmod 600 {PATH}
mkswap {PATH} >/dev/null
swapon {PATH}
# Survives a reboot. Without this the server comes back after any restart with the memory
# it had before, and the next upgrade is killed the way the first install would have been.
grep -q '^{PATH} ' /etc/fstab || printf '%s none swap sw 0 0\\n' {PATH} >> /etc/fstab
echo done"
            ))
            .await?;

        if !said.contains("done") {
            return Err(DeployError::Step {
                id: StepId::Swap,
                detail: said.trim().to_owned(),
                advice: None,
            });
        }
        Ok(())
    })
}
