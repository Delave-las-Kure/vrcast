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

    /// Another change reached the server between this call's read and its write (T600).
    ///
    /// The same idea as `ManifestIoError::Conflict`, adapted to a text config file instead
    /// of JSON: nothing was written, the file on the server is exactly as the OTHER change
    /// left it, and the caller is expected to read again and either retry or give up —
    /// never to treat this as a fault of the serving itself.
    #[error("the rules were changed by another change in between: read generation {base}, server has {current}")]
    Conflict { base: u64, current: u64 },

    #[error(transparent)]
    Ssh(#[from] SshError),
}

/// What the atomic remote compare-and-swap step (T600) found out.
enum CompareAndSwap {
    /// The generation matched; the staged file is now in place.
    Applied,
    /// It did not; nothing on the server was touched, and the staged file was removed by
    /// the script itself.
    Conflict { current: u64 },
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
    /// What limits the **server** says are in force, and the generation they were read at
    /// (T600).
    ///
    /// Read from the server rather than from a note kept here (FR-064): a note goes stale
    /// the moment somebody edits the server by hand, and a list that does not match the
    /// server is worse than no list. The generation comes from the very same read as the
    /// list — a caller that means to write back must pass exactly this number as
    /// `apply()`'s `base_generation`, or the check below is comparing against a read that
    /// never happened.
    pub async fn limits(&self) -> Result<(Vec<Limit>, u64), LimitError> {
        let out = self
            .conn
            .exec(&format!(
                "cat {} 2>/dev/null || true",
                super::shell_quote(self.conf_path)
            ))
            .await?;
        Ok((
            limits_conf::parse(&out.stdout),
            limits_conf::read_generation(&out.stdout),
        ))
    }

    /// Put a set of limits in force, whole.
    ///
    /// The order is not a preference and none of it may be skipped:
    ///
    ///  1. the shortened descriptions are written first — a rule pointing at a description
    ///     that is not there yet would serve a limited viewer nothing at all;
    ///  2. the new rules are staged in a temporary file, not yet in force;
    ///  3. **one atomic remote step** (T600) checks the generation, keeps the previous file,
    ///     and puts the staged one in place — all inside a single `flock`'d shell script, so
    ///     no other `apply()` can interleave between the check and the swap;
    ///  4. the web server is asked to check what is now in place **by its own means** — our
    ///     opinion of a configuration file is worth nothing;
    ///  5. it is reloaded;
    ///  6. the serving is asked for something a viewer would ask for.
    ///
    /// From step 4 onwards, any failure puts the previous file back, reloads, and checks
    /// that the serving works.
    ///
    /// **Why the checking happens after the file is in place and not before.** The main
    /// configuration imports this file by name; until the new content is under that name
    /// there is nothing for the web server to check. That is safe because of something the
    /// web server does rather than something we do: a reload that is refused leaves the
    /// **previous** configuration running. So a bad file is caught while the old one is
    /// still serving.
    ///
    /// **`base_generation` — optimistic concurrency for a text file (T600), made genuinely
    /// atomic rather than read-then-write.** A plain "read the generation, then separately
    /// write if it still matches" is not enough: two truly concurrent calls can each pass
    /// that check before either has written — read and write are three separate SSH
    /// round-trips, and all three of another call's round-trips can land in between any two
    /// of this call's own. Measured directly while writing this fix: a naive read-then-write
    /// version of this exact check still lost a rule silently under `tokio::join!` in
    /// integration testing. The fix is doing the compare **and** the swap in one
    /// [`flock`(1)](https://man7.org/linux/man-pages/man1/flock.1.html)'d shell script run in
    /// a single `exec()` — a single command the remote shell either runs as one atomic unit
    /// while holding the lock, or does not run at all; there is no window between the check
    /// and the write for another call to land in, because there is no round-trip between
    /// them at all.
    ///
    /// `base_generation` must be the generation `Serving::limits()` returned alongside the
    /// list this `limits` argument was built from (`commands/limits.rs::api::limit_set`/
    /// `limit_clear` do exactly this). A mismatch returns `LimitError::Conflict` having
    /// written nothing at all. A match writes `generation = base_generation + 1` — the same
    /// "I am writing over what I read" claim `Manifest::prepared_for_write` makes for the
    /// JSON catalogue.
    pub async fn apply(
        &self,
        limits: &[Limit],
        shortened: &[(String, Shortened)],
        base_generation: u64,
    ) -> Result<(), LimitError> {
        for (slug, short) in shortened {
            self.write_shortened(slug, short).await?;
        }

        // Staged rather than written straight to `conf_path`: the atomic step below moves
        // this into place only if the generation still matches, and a file half-written by
        // `write_file` sitting directly at `conf_path` would be exactly the kind of half
        // state R-10-style staging exists to avoid.
        let temp = format!("{}.{}.tmp", self.conf_path, uuid::Uuid::new_v4().simple());
        let text = limits_conf::build(limits, self.serving_prefix, base_generation + 1);
        self.write_file(&temp, &text).await?;

        let backup = format!("{}.previous", self.conf_path);
        match self.compare_and_swap(&temp, &backup, base_generation).await {
            Ok(CompareAndSwap::Applied) => {}
            Ok(CompareAndSwap::Conflict { current }) => {
                return Err(LimitError::Conflict {
                    base: base_generation,
                    current,
                });
            }
            Err(e) => return Err(e),
        }

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

    /// The one atomic remote step `apply()` rests on (T600): under a single `flock`, keep
    /// the previous file if there is one, check the generation against what this call read,
    /// and only on a match put the staged file in place. All three happen inside one shell
    /// script run by one `exec()` — there is no SSH round-trip between the check and the
    /// swap for a second call to land in.
    ///
    /// On a conflict the staged file is removed by the script itself (`rm -f {temp}`) before
    /// it returns — nothing is left behind for the caller to clean up, and nothing on the
    /// server was touched at all.
    async fn compare_and_swap(
        &self,
        temp: &str,
        backup: &str,
        base_generation: u64,
    ) -> Result<CompareAndSwap, LimitError> {
        let lock_path = format!("{}.lock", self.conf_path);
        // `awk '{print $3}'` on a line shaped `# vrcast-generation N`: field 1 is `#`, field
        // 2 is `vrcast-generation`, field 3 is the number — see `GENERATION_MARK`.
        // `${cur:-0}` is the backward-compatibility rule from `read_generation`'s own doc,
        // reproduced here in shell rather than only in Rust: a file with no such line at all
        // (a server from before T600) reads as generation zero, not as an error.
        let script = format!(
            "(\n\
             flock -x -w 30 200 || {{ echo LOCK_TIMEOUT; exit 4; }}\n\
             cur=$(grep -m1 '^# vrcast-generation ' {conf} 2>/dev/null | awk '{{print $3}}')\n\
             cur=${{cur:-0}}\n\
             if [ \"$cur\" != {base} ]; then\n\
             echo \"CONFLICT $cur\"\n\
             rm -f {temp}\n\
             exit 3\n\
             fi\n\
             if [ -f {conf} ]; then cp -p {conf} {backup}; fi\n\
             mv -f {temp} {conf}\n\
             echo OK\n\
             ) 200>{lock}",
            conf = super::shell_quote(self.conf_path),
            base = super::shell_quote(&base_generation.to_string()),
            temp = super::shell_quote(temp),
            backup = super::shell_quote(backup),
            lock = super::shell_quote(&lock_path),
        );

        let out = self.conn.exec(&script).await?;
        let first_line = out.stdout.lines().next().unwrap_or("").trim();
        if first_line == "OK" {
            return Ok(CompareAndSwap::Applied);
        }
        if let Some(rest) = first_line.strip_prefix("CONFLICT ") {
            let current = rest.trim().parse().unwrap_or(base_generation.wrapping_add(1));
            return Ok(CompareAndSwap::Conflict { current });
        }
        Err(LimitError::Ssh(SshError::Exec(format!(
            "the atomic generation check did not behave as expected: exit {:?}, stdout {:?}, \
             stderr {:?}",
            out.exit_code,
            out.stdout.trim(),
            out.stderr.trim()
        ))))
    }

    /// Take a limit off: the rule and the shortened description both (FR-065).
    ///
    /// The description goes as well as the rule. Left behind it is a file nobody reaches,
    /// which is untidy — and worse, it is a file that would be served again the moment
    /// somebody set a limit on that medium and expected a fresh one.
    pub async fn clear(
        &self,
        remaining: &[Limit],
        slug: &str,
        base_generation: u64,
    ) -> Result<(), LimitError> {
        self.apply(remaining, &[], base_generation).await?;
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
