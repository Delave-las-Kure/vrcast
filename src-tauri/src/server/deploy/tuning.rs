//! T279 — the settings that make the serving fast, all of them measured.
//!
//! Carried over from the skill unchanged (principle VI). Each was bought:
//!
//! - **BBR instead of cubic.** Cubic reads *any* lost packet as congestion and cuts the rate;
//!   on Wi-Fi and mobile the losses are random, so the speed collapsed for nothing. BBR
//!   measures the actual bandwidth and round trip and does not flinch at random loss.
//! - **Bigger buffers.** The defaults — 208 KB of write buffer, 4 MB at the top of tcp_wmem —
//!   are small for fifteen viewers at 9–30 Mbit/s across half a country.
//! - **No slow start after idle.** A player pulls video in bursts: a range, a pause, the next
//!   range. Without this, every burst begins from scratch.
//! - **Readahead of 8 MB.** The default 128 KB ran into virtio's latency, about 7 ms a
//!   request, giving 17 MB/s; with 8 MB the measured figure is 40–60 MB/s on sequential
//!   serving.
//! - **A restart line for Caddy.** Its stock unit has none, so `Restart=no` — a crash means
//!   the serving lies there until somebody notices.
//!
//! **Two traps, both recorded.** `net.core.default_qdisc` only takes effect on interfaces
//! created afterwards, so on a running machine the queueing discipline has to be set on the
//! interface by name as well — otherwise the behaviour before and after a reboot differs, and
//! nobody connects the two. And the disk's name is worked out rather than assumed: `vda` is
//! right on a virtio VPS and on other hardware the rule applies to nothing, silently.
//!
//! **In a container none of this can be established** (T246): `net.core.rmem_max`,
//! `wmem_max` and `default_qdisc` are not per-network-namespace and are invisible inside any
//! container on any host; the congestion algorithm can only be set to one the *host's* kernel
//! carries; and udev has no real block device to apply a rule to.

use futures::future::BoxFuture;

use crate::domain::deploy_steps::{Change, Checked, StepId};

use super::{Context, DeployError, Result, Step};

const SYSCTL: &str = "/etc/sysctl.d/99-vrcast-net.conf";
const UDEV: &str = "/etc/udev/rules.d/60-vrcast-readahead.rules";
const CADDY_DROP_IN: &str = "/etc/systemd/system/caddy.service.d/10-restart.conf";
const BBR_MODULE: &str = "/etc/modules-load.d/bbr.conf";

const NET: &str = include_str!("../../../resources/server/99-vrcast-net.conf");
const READAHEAD: &str = include_str!("../../../resources/server/60-vrcast-readahead.rules");
const RESTART: &str = include_str!("../../../resources/server/caddy-10-restart.conf");

/// What readahead has to come to, in kilobytes. The measured value.
const READAHEAD_KB: u32 = 8192;

/// The readahead rule with this machine's disk in it.
fn readahead_for(disk: &str) -> String {
    READAHEAD.replace("{{DISK}}", disk)
}

pub fn step<'a>() -> Step<Context<'a>> {
    Step {
        id: StepId::Tuning,
        changes,
        check,
        apply,
    }
}

fn changes(_: &Context<'_>) -> Vec<Change> {
    vec![
        Change::SetsKernelSettings,
        Change::WritesFile {
            path: String::from(SYSCTL),
        },
        Change::WritesFile {
            path: String::from(UDEV),
        },
        Change::WritesFile {
            path: String::from(CADDY_DROP_IN),
        },
    ]
}

fn check<'x, 'a>(ctx: &'x Context<'a>) -> BoxFuture<'x, Result<Checked>> {
    Box::pin(async move {
        if ctx.machine.is_container() {
            return Ok(ctx.not_here("the kernel settings and the disk's readahead"));
        }
        let disk = &ctx.machine.disk;
        let interface = &ctx.machine.interface;
        let holds = ctx
            .asks(&format!(
                "ok=1
[ \"$(sysctl -n net.ipv4.tcp_congestion_control 2>/dev/null)\" = bbr ] || ok=0
[ \"$(sysctl -n net.core.default_qdisc 2>/dev/null)\" = fq ] || ok=0
[ \"$(sysctl -n net.ipv4.tcp_slow_start_after_idle 2>/dev/null)\" = 0 ] || ok=0
# The queueing discipline on the interface itself, not only the default. See the note above.
tc qdisc show dev {interface} 2>/dev/null | head -n 1 | grep -q ' fq ' || ok=0
[ \"$(cat /sys/block/{disk}/queue/read_ahead_kb 2>/dev/null)\" = {READAHEAD_KB} ] || ok=0
[ -f {CADDY_DROP_IN} ] || ok=0
[ $ok -eq 1 ] && echo yes || echo no"
            ))
            .await?;
        Ok(if holds {
            Checked::Applied
        } else {
            Checked::NotApplied
        })
    })
}

fn apply<'x, 'a>(ctx: &'x Context<'a>) -> BoxFuture<'x, Result<()>> {
    Box::pin(async move {
        let disk = ctx.machine.disk.clone();
        let interface = ctx.machine.interface.clone();

        ctx.ran("mkdir -p /etc/systemd/system/caddy.service.d /etc/modules-load.d")
            .await?;
        ctx.put_file(SYSCTL, NET).await?;
        ctx.put_file(UDEV, &readahead_for(&disk)).await?;
        ctx.put_file(CADDY_DROP_IN, RESTART).await?;
        ctx.put_file(BBR_MODULE, "tcp_bbr\n").await?;

        let said = ctx
            .ran(&format!(
                "set -e
modprobe tcp_bbr 2>/dev/null || true
sysctl -p {SYSCTL} >/dev/null 2>&1 || true
# The recorded trap: default_qdisc reaches only interfaces made later, so the live one is set
# by name. Without this the machine behaves one way now and another after a reboot.
tc qdisc replace dev {interface} root fq 2>/dev/null || true
udevadm control --reload-rules 2>/dev/null || true
udevadm trigger --subsystem-match=block --sysname-match={disk} 2>/dev/null || true
udevadm settle 2>/dev/null || true
# Belt for the trigger: on some kernels the rule does not reach an already-open device, and
# the value is simply written.
echo {READAHEAD_KB} > /sys/block/{disk}/queue/read_ahead_kb 2>/dev/null || true
systemctl daemon-reload
echo done"
            ))
            .await?;

        if !said.contains("done") {
            return Err(DeployError::Step {
                id: StepId::Tuning,
                detail: said.trim().to_owned(),
                advice: None,
            });
        }
        Ok(())
    })
}
