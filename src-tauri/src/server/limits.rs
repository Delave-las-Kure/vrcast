//! T211, T212, T213, T216 — putting a quality limit on the server, and taking it off again.
//!
//! **The application writes one file and no other** (R-03). The main serving configuration
//! belongs to the person: it may hold things this application knows nothing about, and a
//! mistake in it costs the whole of the serving — including a showing that is happening at
//! that moment. Our rules go into a file the main configuration imports, and that file is
//! the only one ever replaced.
//!
//! **Nothing here is allowed to leave the serving broken** (FR-063). Every step from the
//! checking onwards can put back what was there before, reload, and make sure it works.

use std::time::Duration;

use crate::domain::limits_conf::{self, Limit};
use crate::domain::slow_master::{slow_master_path, Shortened, SLOW_DIR};
use crate::ssh::{Connection, SshError};

/// How long the serving is given to answer after a reload.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Debug, thiserror::Error)]
pub enum LimitError {
    /// The web server refused the configuration. Nothing was changed for anybody.
    #[error("the serving refused the new configuration: {0}")]
    ValidateFailed(String),

    /// The configuration was sound and the reload still failed.
    #[error("the serving would not take the new configuration: {0}")]
    ReloadFailed(String),

    /// The change went in and the serving stopped answering. What was there before is back.
    #[error("the serving stopped answering, so the previous configuration was put back")]
    ServingStopped,

    /// The worst case: the change failed **and** putting the old one back failed too.
    ///
    /// Told apart from the rest on purpose. Everything else leaves a working server and a
    /// person who can try again; this one needs them to go and look.
    #[error("the serving is broken and the previous configuration would not go back: {0}")]
    RollbackFailed(String),

    #[error(transparent)]
    Ssh(#[from] SshError),
}

/// The serving, as far as limits are concerned.
pub struct Serving<'a> {
    pub conn: &'a Connection,
    /// Where the media are, as the person's own profile says.
    pub video_dir: &'a str,
    /// The file this application owns, e.g. `/etc/caddy/vrcast-limits.conf`.
    pub conf_path: &'a str,
    /// The main configuration, only ever read: `/etc/caddy/Caddyfile`.
    pub main_conf: &'a str,
    /// Where the media sit in an address, e.g. `/videos`.
    pub serving_prefix: &'a str,
    /// Something a viewer would ask for, to prove the serving still answers.
    pub check_url: &'a str,
    /// `user:group` the files must belong to.
    pub owner: &'a str,
}

impl Serving<'_> {
    /// What limits the **server** says are in force.
    ///
    /// Read from the server rather than from a note kept here (FR-064): a note goes stale
    /// the moment somebody edits the server by hand, and a list that does not match the
    /// server is worse than no list.
    pub async fn limits(&self) -> Result<Vec<Limit>, LimitError> {
        let out = self
            .conn
            .exec(&format!(
                "cat {} 2>/dev/null || true",
                super::shell_quote(self.conf_path)
            ))
            .await?;
        Ok(limits_conf::parse(&out.stdout))
    }

    /// Put a set of limits in force, whole.
    ///
    /// The order is not a preference and none of it may be skipped:
    ///
    ///  1. the shortened descriptions are written first — a rule pointing at a description
    ///     that is not there yet would serve a limited viewer nothing at all;
    ///  2. the previous rules file is kept;
    ///  3. the new one is put in place and the web server is asked to check it **by its own
    ///     means** — our opinion of a configuration file is worth nothing;
    ///  4. it is reloaded;
    ///  5. the serving is asked for something a viewer would ask for.
    ///
    /// From step 3 onwards, any failure puts the previous file back, reloads, and checks
    /// that the serving works.
    ///
    /// **Why the checking happens after the file is in place and not before.** The main
    /// configuration imports this file by name; until the new content is under that name
    /// there is nothing for the web server to check. That is safe because of something the
    /// web server does rather than something we do: a reload that is refused leaves the
    /// **previous** configuration running. So a bad file is caught while the old one is
    /// still serving.
    pub async fn apply(
        &self,
        limits: &[Limit],
        shortened: &[(String, Shortened)],
    ) -> Result<(), LimitError> {
        for (slug, short) in shortened {
            self.write_shortened(slug, short).await?;
        }

        let backup = format!("{}.previous", self.conf_path);
        // Kept before anything is touched (FR-095). The rollback takes **this file** rather
        // than assembling what it thinks used to be there: what it thinks and what is there
        // are two different things, and the difference only shows up when it matters.
        self.conn
            .exec(&format!(
                "if [ -f {conf} ]; then cp -p {conf} {backup}; fi",
                conf = super::shell_quote(self.conf_path),
                backup = super::shell_quote(&backup),
            ))
            .await?
            .require_ok("could not keep the previous rules")?;

        let text = limits_conf::build(limits, self.serving_prefix);
        self.write_file(self.conf_path, &text).await?;

        if let Err(e) = self.check_and_reload().await {
            self.roll_back(&backup).await?;
            return Err(e);
        }
        if !self.serving_answers().await {
            self.roll_back(&backup).await?;
            return Err(LimitError::ServingStopped);
        }
        Ok(())
    }

    /// Take a limit off: the rule and the shortened description both (FR-065).
    ///
    /// The description goes as well as the rule. Left behind it is a file nobody reaches,
    /// which is untidy — and worse, it is a file that would be served again the moment
    /// somebody set a limit on that medium and expected a fresh one.
    pub async fn clear(&self, remaining: &[Limit], slug: &str) -> Result<(), LimitError> {
        self.apply(remaining, &[]).await?;
        self.conn
            .exec(&format!(
                "rm -f {}",
                super::shell_quote(&slow_master_path(self.video_dir, slug))
            ))
            .await?;
        Ok(())
    }

    /// Write one shortened description where the rule will point.
    pub async fn write_shortened(&self, slug: &str, short: &Shortened) -> Result<(), LimitError> {
        let dir = format!("{}/{SLOW_DIR}/{slug}", self.video_dir.trim_end_matches('/'));
        self.conn
            .exec(&format!("mkdir -p {}", super::shell_quote(&dir)))
            .await?
            .require_ok("could not make room for the shortened description")?;
        self.write_file(&slow_master_path(self.video_dir, slug), &short.text)
            .await?;
        self.conn
            .exec(&format!(
                "chown -R {owner} {dir} 2>/dev/null || true; chmod 644 {file}",
                owner = super::shell_quote(self.owner),
                dir = super::shell_quote(&dir),
                file = super::shell_quote(&slow_master_path(self.video_dir, slug)),
            ))
            .await?;
        Ok(())
    }

    /// Ask the web server to check the configuration, then to take it.
    async fn check_and_reload(&self) -> Result<(), LimitError> {
        let validate = self
            .conn
            .exec(&format!(
                "caddy validate --config {} --adapter caddyfile 2>&1",
                super::shell_quote(self.main_conf)
            ))
            .await?;
        if !validate.ok() {
            return Err(LimitError::ValidateFailed(last_words(&validate.stdout)));
        }

        let reload = self
            .conn
            .exec(&format!(
                "caddy reload --config {} --adapter caddyfile 2>&1",
                super::shell_quote(self.main_conf)
            ))
            .await?;
        if !reload.ok() {
            return Err(LimitError::ReloadFailed(last_words(&reload.stdout)));
        }
        Ok(())
    }

    /// Put back what was there, and make sure the serving works again.
    async fn roll_back(&self, backup: &str) -> Result<(), LimitError> {
        let restored = self
            .conn
            .exec(&format!(
                "if [ -f {backup} ]; then mv {backup} {conf}; else : > {conf}; fi && \
                 caddy reload --config {main} --adapter caddyfile 2>&1",
                backup = super::shell_quote(backup),
                conf = super::shell_quote(self.conf_path),
                main = super::shell_quote(self.main_conf),
            ))
            .await?;
        if !restored.ok() {
            return Err(LimitError::RollbackFailed(last_words(&restored.stdout)));
        }
        if !self.serving_answers().await {
            return Err(LimitError::RollbackFailed(String::from(
                "the previous configuration went back and the serving still does not answer",
            )));
        }
        Ok(())
    }

    /// Ask the serving for something a viewer would ask for.
    ///
    /// From here, over the address a viewer uses — not from the server with a local
    /// request. A file can be on disk and readable and still not be served.
    async fn serving_answers(&self) -> bool {
        let Ok(client) = reqwest::Client::builder().timeout(ANSWER_TIMEOUT).build() else {
            return false;
        };
        match client.get(self.check_url).send().await {
            Ok(answer) => answer.status().is_success(),
            Err(_) => false,
        }
    }

    async fn write_file(&self, path: &str, body: &str) -> Result<(), LimitError> {
        use tokio::io::AsyncWriteExt;

        let sftp = self.conn.sftp().await?;
        let written = async {
            let mut file = sftp.create(path.to_owned()).await?;
            file.write_all(body.as_bytes()).await?;
            file.flush().await?;
            file.shutdown().await?;
            Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
        }
        .await;
        written
            .map_err(|e| LimitError::Ssh(SshError::sftp(crate::store::redact::safe_display(&*e))))
    }
}

/// The last few lines of a complaint — the part that says what is wrong.
///
/// Caddy prints its startup chatter before the actual objection, and a person shown the
/// chatter learns nothing.
fn last_words(text: &str) -> String {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    lines
        .iter()
        .rev()
        .take(3)
        .rev()
        .copied()
        .collect::<Vec<_>>()
        .join("\n")
}
