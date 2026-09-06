//! T282–T285 — bringing an already deployed server up to the version we know.
//!
//! **An upgrade is the same run of the same steps against a newer reference.** That falls out
//! of the mechanism rather than being arranged: every step's check looks at the server and
//! says whether the thing is already so, so pointing those checks at a server deployed by an
//! older version names exactly what has changed. There is no second engine and no list of
//! migrations to keep in step with the steps themselves — a list that would go stale the first
//! time somebody edited a step and forgot it.
//!
//! Two promises hang on this file and neither is decoration:
//!
//! - **nothing raised is lost** (FR-131, SC-017): the videos and the catalogue are not touched
//!   at all, by anything here;
//! - **what is replaced can be put back** (FR-133): every file the application owns is copied
//!   aside first, and one command restores it.

use crate::domain::deploy_steps::{PlannedStep, Status};
use crate::domain::server_state::APP_EXPECTS;
use crate::server::deploy::{self, Context, DeployError, Result, Step};

/// Where copies go before an upgrade replaces anything.
const BACKUP_ROOT: &str = "/etc/vrcast/backup";
/// The one a rollback restores: the last upgrade's copies.
const LATEST: &str = "/etc/vrcast/backup/latest";

/// Every file a deployment writes or changes, and therefore copies aside first.
///
/// **The video directory and the catalogue are deliberately absent.** They are the person's
/// work, not our configuration; an upgrade has no business copying them aside, and no line of
/// this file may put them back either — a restore that "helpfully" reverted the catalogue
/// would undo whatever was uploaded since (FR-131).
const OWNED: [&str; 11] = [
    "/etc/caddy/Caddyfile",
    "/etc/caddy/vrcast-limits.conf",
    "/etc/vrcast/state.json",
    "/etc/sysctl.d/99-vrcast-net.conf",
    "/etc/sysctl.d/99-vrcast-ipv6.conf",
    "/etc/udev/rules.d/60-vrcast-readahead.rules",
    "/etc/systemd/system/caddy.service.d/10-restart.conf",
    "/etc/ssh/sshd_config.d/00-vrcast.conf",
    // ⚠ **Four the application does not own and changes anyway** (T513, T520). The list above
    // is what we write whole; these are somebody else's files that a deployment edits, and
    // FR-095 asks for a copy of the settings files that are **changed**, not of the files that
    // are ours. They were changed by every run and copied by none, so there was nothing to
    // put back.
    //
    // `/etc/fail2ban/jail.local` is the sharpest of them: it is written whole, over whatever
    // an owner had there. The other three are edited in place or appended to, which is gentler
    // and no less irreversible without a copy.
    //
    // A file that is not there when the copy is taken is simply skipped — `back_up` guards
    // each one with `[ -e "$f" ]` — so on a truly bare server this costs nothing and saves
    // nothing, which is right: there was nothing to lose.
    "/etc/default/ufw",
    "/etc/fstab",
    "/etc/fail2ban/jail.local",
];

/// What an upgrade would do (FR-129).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Plan {
    /// The version on the server now.
    pub from: u32,
    /// The version this application deploys.
    pub to: u32,
    /// Every step, with what its check found. The ones marked not applied are the change.
    pub steps: Vec<PlannedStep>,
    /// What will be copied aside before anything is replaced.
    pub backing_up: Vec<String>,
}

impl Plan {
    /// Is there anything to do?
    ///
    /// Asked of the checks and not of the version numbers. A server whose version is current
    /// can still have drifted — a file edited, a service disabled — and saying "nothing to do"
    /// because a number matched would leave it that way.
    pub fn has_work(&self) -> bool {
        self.steps
            .iter()
            .any(|s| matches!(s.status, Status::NotApplied))
    }
}

/// Ask what an upgrade would change, and change nothing.
pub async fn plan<'a>(ctx: &Context<'a>, from: u32, steps: &[Step<Context<'a>>]) -> Result<Plan> {
    Ok(Plan {
        from,
        to: APP_EXPECTS,
        steps: deploy::plan(ctx, steps).await?,
        backing_up: OWNED.iter().map(|p| String::from(*p)).collect(),
    })
}

/// Copy aside everything that may be replaced (FR-133).
///
/// Done **before** the first change and not as the run goes: copies made along the way are
/// half a backup, and half a backup restores a server into a state it was never in.
pub async fn back_up(ctx: &Context<'_>) -> Result<String> {
    let stamp = ctx.ran("date -u +%Y%m%dT%H%M%SZ").await?.trim().to_owned();
    if stamp.is_empty() {
        return Err(DeployError::Ssh(crate::ssh::SshError::sftp(String::from(
            "the server would not say what time it is",
        ))));
    }
    let dir = format!("{BACKUP_ROOT}/{stamp}");
    let files = OWNED.join(" ");

    ctx.ran(&format!(
        "set -e
mkdir -p {dir}
for f in {files}; do
  # Missing files are skipped rather than faked: a backup holding an empty stand-in for a
  # file that did not exist would, on restore, create it — and a configuration file that
  # appears from nowhere is worse than one that is absent.
  [ -e \"$f\" ] && cp -a \"$f\" {dir}/ || true
done
ln -sfn {dir} {LATEST}
echo done"
    ))
    .await?;
    Ok(dir)
}

/// Carry the upgrade out.
pub async fn run<'a>(
    ctx: &Context<'a>,
    steps: &[Step<Context<'a>>],
    cancelled: &(dyn Fn() -> bool + Sync),
    watch: &mut (dyn FnMut(&PlannedStep) + Send),
) -> Result<Vec<PlannedStep>> {
    back_up(ctx).await?;
    deploy::run(ctx, steps, cancelled, watch).await
}

/// Where each backed-up file goes back to, built from [`OWNED`].
///
/// ⚠ **This used to be a second list, written out by hand beside the first.** Eight paths in
/// `OWNED` and eight arms of a `case`, kept in step by nobody: add a file to `OWNED` and
/// forget the arm, and it is copied aside faithfully and never put back — a rollback that
/// reports success having restored less than it saved. That is the loophole `str_enum!` was
/// written to close for the enumerations, in a place where nothing closed it.
///
/// Made from the one list instead. The `mkdir -p` is unconditional rather than only for the
/// one nested directory that needed it: a directory that already exists costs nothing, and an
/// arm that forgets it is the same bug in miniature.
pub fn restore_arms() -> String {
    OWNED
        .iter()
        .map(|path| {
            let name = path.rsplit('/').next().unwrap_or(path);
            let dir = path.rsplit_once('/').map(|(d, _)| d).unwrap_or("/");
            format!("    {name}) mkdir -p {dir} && cp -a \"$f\" {path} ;;")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Put the last backup back (FR-133).
///
/// The services are reloaded afterwards rather than restarted: a restart drops every viewer,
/// and somebody rolling an upgrade back is already having a bad enough time.
pub async fn roll_back(ctx: &Context<'_>) -> Result<()> {
    let there = ctx
        .asks(&format!("test -d {LATEST} && echo yes || echo no"))
        .await?;
    if !there {
        return Err(DeployError::Ssh(crate::ssh::SshError::sftp(String::from(
            "there is nothing to roll back to",
        ))));
    }

    let said = ctx
        .ran(&format!(
            "set -e
for f in {LATEST}/*; do
  [ -e \"$f\" ] || continue
  name=$(basename \"$f\")
  case \"$name\" in
{arms}
  esac
done
systemctl daemon-reload 2>/dev/null || true
systemctl reload caddy 2>/dev/null || true
sshd -t && (systemctl reload ssh 2>/dev/null || systemctl reload sshd 2>/dev/null) || true
echo done",
            arms = restore_arms()
        ))
        .await?;

    if !said.contains("done") {
        return Err(DeployError::Ssh(crate::ssh::SshError::sftp(
            said.trim().to_owned(),
        )));
    }
    Ok(())
}

/// What the application owns on a server. Public so a check can assert that the list and the
/// contract agree — the ownership rule is the one thing that keeps a deployment from being
/// able to break somebody's machine.
pub fn owned_files() -> Vec<String> {
    OWNED.iter().map(|p| String::from(*p)).collect()
}
