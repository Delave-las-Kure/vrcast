//! T274 — turning password logins off, and knowing that they are off.
//!
//! **The six-month mistake lives here.** On the live server this step was written, it ran
//! without complaint, and for half a year `sshd -T` answered that password authentication was
//! allowed — while twenty-two thousand attempts a day went at it. What was checked was that
//! the right line was in a file. A line in a file proves that the line is in the file.
//!
//! So two things are asked instead: the **effective** configuration, which is `sshd -T` and
//! not the file, and a **real login attempt** with a password, which is the only thing that
//! settles it.
//!
//! And because the step can lock the application out of the server it is configuring, it arms
//! an undo before it changes anything: a detached job that puts the old configuration back
//! unless it is told, within a few minutes, that the new way in works. Left armed by a
//! failure, it fires and the server recovers on its own.

use futures::future::BoxFuture;

use crate::domain::deploy_steps::{Change, Checked, StepId};

use super::{Context, DeployError, Result, Step};

/// Our own drop-in. The distribution's files are left alone: they belong to the distribution,
/// and a deployment that edits them fights every future upgrade of the package.
///
/// **The number is 00 and it is the whole of whether this step works.** Measured 2026-08-27:
/// sshd reads `sshd_config.d/*.conf` in lexical order and, for each keyword, **the first
/// value it meets wins**. A provider's image enables password logins from a drop-in of its
/// own — `50-cloud-init.conf` on Ubuntu — so a file of ours numbered 99 is read second and
/// changes nothing at all, while `sshd -T` goes on answering `passwordauthentication yes`.
///
/// That is very likely the mechanism behind the six months of it on the live server: the
/// skill writes `99-vrcast.conf` and separately edits the cloud-init file with a `sed` whose
/// errors are sent to /dev/null — and if that edit does not match, nothing says so.
///
/// Sorting first rather than editing somebody else's file: their file goes on being theirs,
/// survives its package's upgrades, and our value still wins.
const DROP_IN: &str = "/etc/ssh/sshd_config.d/00-vrcast.conf";

/// How long the undo waits before putting the old configuration back.
///
/// **A choice, not a measurement.** Five minutes: long enough for a fresh connection to be
/// made and answered over a slow link, short enough that a person watching a failed
/// deployment does not conclude the server is gone. Too short and it would undo a setting
/// that was working; too long and a locked-out owner sits waiting.
const UNDO_AFTER_SECONDS: u32 = 300;

/// The flag that calls the undo off.
const OK_FLAG: &str = "/root/.vrcast-hardening-ok";
const BACKUP: &str = "/root/.vrcast-sshd-before";

const WANTED: &str = "\
# Written by VRCast Studio. Password logins are off; the way in is the key.
PasswordAuthentication no
PermitRootLogin prohibit-password
KbdInteractiveAuthentication no
";

pub fn step<'a>() -> Step<Context<'a>> {
    Step {
        id: StepId::SshHardening,
        changes,
        check,
        apply,
    }
}

fn changes(_: &Context<'_>) -> Vec<Change> {
    vec![Change::TurnsPasswordLoginOff]
}

fn check<'x, 'a>(ctx: &'x Context<'a>) -> BoxFuture<'x, Result<Checked>> {
    Box::pin(async move {
        // `sshd -T` — the configuration in force, worked out by sshd itself from every file it
        // reads and in the order it reads them. Not our drop-in, which a later file can and
        // does override.
        let effective = ctx
            .asks(
                "sshd -T 2>/dev/null | grep -qx 'passwordauthentication no' \\
                 && sshd -T 2>/dev/null | grep -qE '^permitrootlogin (prohibit-password|without-password|no)$' \\
                 && echo yes || echo no",
            )
            .await?;
        if !effective {
            return Ok(Checked::NotApplied);
        }
        // And the thing itself. Everything above can be true of a server that still lets a
        // password in — through PAM, through a match block, through a second sshd.
        Ok(if (ctx.proofs.password_refused)().await {
            Checked::Applied
        } else {
            Checked::NotApplied
        })
    })
}

fn apply<'x, 'a>(ctx: &'x Context<'a>) -> BoxFuture<'x, Result<()>> {
    Box::pin(async move {
        // The key has to work before the password goes. The step before proved it, and it is
        // proved again here: the two steps may be separated by a failed run, a restart of the
        // application, or a person who edited something in between.
        if !(ctx.proofs.key_works)().await {
            return Err(DeployError::Step {
                id: StepId::SshHardening,
                detail: String::from(
                    "the key does not let us in, so the password must not be turned off",
                ),
                advice: None,
            });
        }

        // Arm the undo first. Everything after this point is reversible without the person
        // touching a console.
        ctx.ran(&format!(
            "set -e
rm -f {OK_FLAG}
rm -rf {BACKUP}
mkdir -p {BACKUP}
cp -a /etc/ssh/sshd_config {BACKUP}/ 2>/dev/null || true
cp -a /etc/ssh/sshd_config.d {BACKUP}/ 2>/dev/null || true
setsid nohup sh -c 'sleep {UNDO_AFTER_SECONDS}
if [ ! -f {OK_FLAG} ]; then
  cp -a {BACKUP}/sshd_config /etc/ssh/sshd_config 2>/dev/null || true
  rm -rf /etc/ssh/sshd_config.d
  cp -a {BACKUP}/sshd_config.d /etc/ssh/ 2>/dev/null || true
  systemctl reload ssh 2>/dev/null || systemctl reload sshd 2>/dev/null || true
fi' >/dev/null 2>&1 &
echo armed"
        ))
        .await?;

        ctx.put_file(DROP_IN, WANTED).await?;

        let said = ctx
            .ran(
                "sshd -t 2>&1 && (systemctl reload ssh 2>/dev/null || systemctl reload sshd) && echo done",
            )
            .await?;
        if !said.contains("done") {
            return Err(DeployError::Step {
                id: StepId::SshHardening,
                detail: format!("{} (the undo is armed and will put it back)", said.trim()),
                advice: None,
            });
        }

        // **The proof, and only then the flag.** Calling the undo off before knowing the new
        // way in works is the same as not arming it.
        if !(ctx.proofs.password_refused)().await {
            return Err(DeployError::Step {
                id: StepId::SshHardening,
                detail: String::from(
                    "the configuration says password logins are off and a password still gets in",
                ),
                advice: None,
            });
        }
        if !(ctx.proofs.key_works)().await {
            return Err(DeployError::Step {
                id: StepId::SshHardening,
                detail: String::from("the key stopped working after the change"),
                advice: None,
            });
        }

        ctx.ran(&format!("touch {OK_FLAG}")).await?;
        Ok(())
    })
}
