//! T269 — what has to be installed (FR-121).
//!
//! The list is in `resources/server/versions.json`, which is also the composition version 1
//! of the server side is frozen at (T252, T337). MediaMTX is deliberately not in it: the
//! application never once went that way, the serving is direct files and segments through
//! Caddy, and keeping a service nobody uses means installing, versioning, repairing and
//! explaining its failures for ever.
//!
//! **Nothing here may ask a question.** An interactive prompt on a server nobody is looking
//! at is a deployment that has hung for good, and it hangs in the middle — after the packages
//! are half unpacked.

use futures::future::BoxFuture;

use crate::domain::deploy_steps::{Change, Checked, StepId};

use super::{Context, DeployError, Result, Step};

/// What is installed from the distribution's own archives.
/// Public so `versions.json` can be checked against **this** list rather than a copy of it
/// beside the check (T337). A copy goes stale the first time a package is added, and the
/// check then passes while guarding a composition nobody deploys.
pub const FROM_APT: [&str; 6] = ["ffmpeg", "curl", "tar", "ufw", "ca-certificates", "gnupg"];

/// Caddy comes from its own repository rather than as a pinned archive with a checksum.
///
/// The reason is FR-126: a repository is covered by the automatic security updates the
/// deployment turns on, and a pinned archive is cut off from them — every patch would need an
/// upgrade of the server side. What is checked instead is the floor: a repository may hand out
/// anything, and silently taking whatever came back is not a foundation.
const CADDY_KEY: &str = "https://dl.cloudsmith.io/public/caddy/stable/gpg.key";
const CADDY_LIST: &str = "https://dl.cloudsmith.io/public/caddy/stable/debian.deb.txt";

pub fn step<'a>() -> Step<Context<'a>> {
    Step {
        id: StepId::Packages,
        changes,
        check,
        apply,
    }
}

fn changes(_: &Context<'_>) -> Vec<Change> {
    let mut names: Vec<String> = FROM_APT.iter().map(|s| String::from(*s)).collect();
    names.push(String::from("caddy"));
    vec![Change::InstallsPackages { names }]
}

fn check<'x, 'a>(ctx: &'x Context<'a>) -> BoxFuture<'x, Result<Checked>> {
    Box::pin(async move {
        // Asked of dpkg rather than of `command -v`: a binary on the path may have been put
        // there by hand, and the deployment's promise is that these are managed packages
        // which the automatic security updates will keep patched.
        let names = FROM_APT.join(" ");
        let all_there = ctx
            .asks(&format!(
                "missing=0
for p in {names} caddy; do
  dpkg-query -W -f='${{Status}}' \"$p\" 2>/dev/null | grep -q 'ok installed' || missing=1
done
[ $missing -eq 0 ] && echo yes || echo no"
            ))
            .await?;
        Ok(if all_there {
            Checked::Applied
        } else {
            Checked::NotApplied
        })
    })
}

fn apply<'x, 'a>(ctx: &'x Context<'a>) -> BoxFuture<'x, Result<()>> {
    Box::pin(async move {
        let names = FROM_APT.join(" ");
        let said = ctx
            .ran(&format!(
                "set -e
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq
apt-get install -y -qq {names}
if ! command -v caddy >/dev/null; then
  curl -1sLf {CADDY_KEY} | gpg --dearmor -o /usr/share/keyrings/caddy-stable-archive-keyring.gpg
  curl -1sLf {CADDY_LIST} > /etc/apt/sources.list.d/caddy-stable.list
  apt-get update -qq
  apt-get install -y -qq caddy
fi
echo done"
            ))
            .await?;

        if !said.contains("done") {
            return Err(DeployError::Step {
                id: StepId::Packages,
                detail: said.trim().to_owned(),
                advice: None,
            });
        }
        Ok(())
    })
}
