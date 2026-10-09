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

use super::{Context, Result, Step, APT_GET};

/// What is installed from the distribution's own archives.
/// Public so `versions.json` can be checked against **this** list rather than a copy of it
/// beside the check (T337). A copy goes stale the first time a package is added, and the
/// check then passes while guarding a composition nobody deploys.
pub const FROM_APT: [&str; 6] = ["ffmpeg", "curl", "tar", "ufw", "ca-certificates", "gnupg"];

/// T713 — Caddy comes from its own GitHub release, at this version, checked by its sum.
///
/// ⚠ **It used to come from Caddy's apt repository at dl.cloudsmith.io**, for FR-126: a
/// repository is covered by the automatic security updates and a pinned archive is not. On
/// 2026-10-09 that repository started answering `402 Payment Required` to `InRelease` and
/// `Release`, and every fresh deployment failed in this step with apt's «Failed to fetch».
/// The owner's decision (T713): the release package from GitHub, pinned, its SHA-256 checked
/// before it is installed. The price is the one FR-126 named — a Caddy patch now reaches a
/// server with an upgrade of the server side, not on its own.
///
/// The same package the repository handed out (built by the same release pipeline: the
/// systemd unit, the `caddy` user made by its own `postinst`, the Caddyfile a dpkg conffile).
/// Public, with [`CADDY_DEBS`], so `versions.json` is checked against what is installed.
pub const CADDY_VERSION: &str = "2.11.7";

/// The release package's SHA-256 for each machine kind Caddy publishes one for, by Debian's
/// name for it (`dpkg --print-architecture`). Taken from the packages themselves and checked
/// against the release's own `caddy_<v>_checksums.txt` (SHA-512) when the pin was set.
pub const CADDY_DEBS: [(&str, &str); 2] = [
    (
        "amd64",
        "a22b914ffd1958da42bc7ab13b7b62c6100634e0798ab594891d2d61d53ba749",
    ),
    (
        "arm64",
        "ca8ae5a50439642d7672344b4ec64e47cbddf81beb5da302420a78522eb5a47e",
    ),
];

/// Where the releases are; the package is `v<V>/caddy_<V>_linux_<arch>.deb` under it.
pub const CADDY_RELEASES: &str = "https://github.com/caddyserver/caddy/releases/download";

/// What version 1 put on a server to reach the old repository (T713). Taken off by this step:
/// left there, the repository's 402 fails **every** `apt-get update` on the machine — this
/// step's on an upgrade, and the automatic security updates' own every night.
pub const OLD_LIST: &str = "/etc/apt/sources.list.d/caddy-stable.list";

/// The old repository's signing key, the path its list file names. Removed with the list.
/// Kept as a name because a version-1 run interrupted while writing it left its temp copy
/// behind, and the tidy-up of every run still looks for that (T622).
pub const KEYRING: &str = "/usr/share/keyrings/caddy-stable-archive-keyring.gpg";

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

/// Is a managed Caddy of at least the pinned version there? A shell group that prints
/// nothing: its exit status is the answer. Newer is accepted — a Caddy somebody brought past
/// the pin is not ours to take back down.
pub fn caddy_is_current() -> String {
    format!(
        "{{ dpkg-query -W -f='${{Status}}' caddy 2>/dev/null | grep -q 'ok installed' \
&& dpkg --compare-versions \"$(dpkg-query -W -f='${{Version}}' caddy 2>/dev/null)\" ge {CADDY_VERSION}; }}"
    )
}

/// The shell block that puts the pinned Caddy on (T713) when [`caddy_is_current`] says no.
/// Public so its shape is checked without a server.
///
/// Its failures are written the way apt writes its own (`E: …` on standard error), so the
/// step fails with them through `ctx.apt` like any other install.
pub fn caddy_install_script() -> String {
    let arms = CADDY_DEBS
        .iter()
        .map(|(arch, sum)| format!("    {arch}) sum={sum} ;;"))
        .collect::<Vec<_>>()
        .join("\n");
    let current = caddy_is_current();
    format!(
        "if ! {current}; then
  arch=$(dpkg --print-architecture)
  case \"$arch\" in
{arms}
    *) echo \"E: Caddy {CADDY_VERSION} has no release package for this machine ($arch)\" >&2; exit 1 ;;
  esac
  had=no
  if systemctl is-active --quiet caddy 2>/dev/null; then had=yes; fi
  d=$(mktemp -d)
  trap 'rm -rf \"$d\"' EXIT
  deb=\"$d/caddy_{CADDY_VERSION}_linux_$arch.deb\"
  url=\"{CADDY_RELEASES}/v{CADDY_VERSION}/caddy_{CADDY_VERSION}_linux_$arch.deb\"
  if ! curl -fsSL --retry 5 --retry-delay 3 --connect-timeout 20 -o \"$deb\" \"$url\"; then
    echo \"E: Failed to fetch $url\" >&2; exit 1
  fi
  if ! echo \"$sum  $deb\" | sha256sum -c --quiet - >/dev/null 2>&1; then
    echo \"E: the Caddy package from $url is not the one expected (its SHA-256 differs)\" >&2; exit 1
  fi
  # The Caddyfile already there is kept: the package's own is a conffile, and dpkg would
  # otherwise ask what to do with it, on a server nobody is looking at.
  dpkg --force-confdef --force-confold -i \"$deb\" >/dev/null
  # A Caddy that was serving goes on running the old program until it is started again.
  if [ \"$had\" = yes ]; then systemctl try-restart caddy; fi
  rm -rf \"$d\"
  trap - EXIT
fi"
    )
}

fn check<'x, 'a>(ctx: &'x Context<'a>) -> BoxFuture<'x, Result<Checked>> {
    Box::pin(async move {
        // Asked of dpkg rather than of `command -v`: a binary on the path may have been put
        // there by hand, and the deployment's promise is that these are managed packages.
        //
        // T713: and the old repository's list is gone. A version-1 server has every package
        // and still has to come this way, or its `apt-get update` fails on the 402 for good.
        let names = FROM_APT.join(" ");
        let current = caddy_is_current();
        let all_there = ctx
            .asks(&format!(
                "missing=0
for p in {names}; do
  dpkg-query -W -f='${{Status}}' \"$p\" 2>/dev/null | grep -q 'ok installed' || missing=1
done
{current} || missing=1
[ -e {OLD_LIST} ] && missing=1
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
        // T713: the old repository's list and key go first, before the `update` they would
        // fail — and with them the key's temp copy an interrupted version-1 run left.
        //
        // The apt work itself goes through `ctx.apt` (T614): retries on the downloads, the
        // interrupted-dpkg repair first (T609), and apt's own words when it fails.
        ctx.apt(
            StepId::Packages,
            &format!(
                "rm -f -- {OLD_LIST} {KEYRING} {KEYRING}.vrcast.tmp
{APT_GET} update -qq
{APT_GET} install -y -qq {names}
{caddy}",
                caddy = caddy_install_script()
            ),
        )
        .await
    })
}
