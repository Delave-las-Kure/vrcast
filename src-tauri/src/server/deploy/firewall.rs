//! T275 — only what is needed is open, and equally over both families (FR-126, FR-136).
//!
//! **A recorded trap:** `systemctl is-active ufw` says `inactive` on a perfectly protected
//! server. It is a one-shot unit — it puts its rules into the kernel and exits — so the truth
//! is in `ufw status` and in the rules that are actually loaded. A check that did not know
//! this would call every protected server a problem, and a warning that is always there
//! teaches people not to read warnings.
//!
//! Both families are checked, not just IPv4. FR-136 asks for parity, and the failure it
//! prevents is quiet: a server closed over IPv4 and wide open over IPv6 looks protected in
//! every ordinary check.

use futures::future::BoxFuture;

use crate::domain::deploy_steps::{Change, Checked, StepId};

use super::{Context, DeployError, Result, Step};

/// What may be reached from outside. Nothing else.
const PORTS: [&str; 4] = ["22/tcp", "80/tcp", "443/tcp", "443/udp"];

pub fn step<'a>() -> Step<Context<'a>> {
    Step {
        id: StepId::Firewall,
        changes,
        check,
        apply,
    }
}

fn changes(_: &Context<'_>) -> Vec<Change> {
    vec![
        Change::OpensPorts {
            ports: PORTS.iter().map(|p| String::from(*p)).collect(),
        },
        Change::ClosesEverythingElse,
    ]
}

fn check<'x, 'a>(ctx: &'x Context<'a>) -> BoxFuture<'x, Result<Checked>> {
    Box::pin(async move {
        let ports = PORTS.join(" ");
        let holds = ctx
            .asks(&format!(
                "ok=1
# NOT `systemctl is-active ufw`: that says inactive on a protected server, because ufw is a
# one-shot unit. The truth is here.
ufw status 2>/dev/null | head -n 1 | grep -q 'Status: active' || ok=0
for p in {ports}; do
  ufw status 2>/dev/null | grep -q \"^$p\" || ok=0
done
# The parity FR-136 asks for. A server closed over IPv4 and open over IPv6 passes every
# ordinary check and is not protected.
[ \"$(iptables -S 2>/dev/null | grep -c ufw)\" -gt 0 ] || ok=0
[ \"$(ip6tables -S 2>/dev/null | grep -c ufw)\" -gt 0 ] || ok=0
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
        let allows: String = PORTS
            .iter()
            .map(|p| format!("ufw allow {p} >/dev/null\n"))
            .collect();
        let said = ctx
            .ran(&format!(
                "set -e
# The rule for our own way in goes first. `ufw --force enable` closes everything not allowed,
# and doing that before allowing 22 would end the connection this command is running over.
ufw allow 22/tcp >/dev/null
ufw default deny incoming >/dev/null
ufw default allow outgoing >/dev/null
{allows}\
# IPv6 handled by ufw itself rather than by a second set of rules: one file, one truth, and
# no way for the two families to drift apart.
sed -i 's/^IPV6=.*/IPV6=yes/' /etc/default/ufw 2>/dev/null || true
ufw --force enable >/dev/null
echo done"
            ))
            .await?;

        if !said.contains("done") {
            return Err(DeployError::Step {
                id: StepId::Firewall,
                detail: said.trim().to_owned(),
                advice: None,
            });
        }
        Ok(())
    })
}
