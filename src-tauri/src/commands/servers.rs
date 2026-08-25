//! T040–T042 — the commands that manage server profiles.
//!
//! The contract: `contracts/ipc-commands.md`, the "Servers" section.
//!
//! The main rule of this section: **a secret crosses the boundary exactly once** — when
//! the interface hands it over while creating or changing a profile. It never comes back,
//! by any command (FR-090, FR-091). So the answers here have no field for a secret and
//! cannot have one: what comes back is a `ServerProfile`, holding only a pointer to an entry
//! in the operating system's store.

use super::error::{AppError, DetailCode, ErrorCode, Result};
use super::AppState;
use crate::domain::server_profile::{AuthKind, Ipv6Mode, ServerProfile};
use crate::domain::wording::Detail;
use serde::{Deserialize, Serialize};

/// A profile's fields in the form the interface sends them.
///
/// A type of its own, apart from [`ServerProfile`], deliberately: the interface sets
/// neither the profile's identifier, nor the pointer to the secret, nor the fingerprint —
/// the core fills those in. Accepting a whole profile would let the interface substitute
/// another profile's pointer to a secret.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerInput {
    pub name: String,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub auth_kind: AuthKind,
    /// Only when logging in by key.
    pub key_path: Option<String>,
    pub domain: String,
    /// Empty means the default serving directory.
    pub video_dir: Option<String>,
    pub cdn_base: Option<String>,
    pub ipv6_mode: Option<Ipv6Mode>,
}

/// How one step of the connection check went.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    Ok,
    Failed,
    /// The step did not run: we stopped earlier. It is shown alongside the rest — a person
    /// needs to see exactly where things broke off (FR-003).
    Skipped,
}

/// One step of the connection check.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestStep {
    /// Stable step name: `network`, `login`, `video_dir`, `domain`. The interface
    /// looks up its title by this, so the title no longer travels with every step.
    pub id: String,
    pub status: StepStatus,
    /// What to say about the outcome: what the server answered, what was missing.
    pub detail: Option<Detail>,
}

/// The order of the check's steps. Also the order they are shown in.
///
/// The order is not arbitrary: each step makes sense only when the one before it succeeded.
/// Checking that a domain serves content without having managed to log in to the server
/// would tell a person about the second trouble without naming the first.
pub const TEST_STEPS: [&str; 4] = ["network", "login", "video_dir", "domain"];

/// A suggestion to carry the settings over from `server.env` (T043).
#[derive(Debug, Clone, Serialize)]
pub struct ImportSuggestion {
    /// Where it came from — a person must understand what was filled in and from where.
    pub source: String,
    /// There is a key but no passphrase in the file: it will have to be typed in.
    pub needs_passphrase: bool,
    pub input: ServerInput,
}

/// Build a profile out of the fields that were sent.
///
/// The identifier, the pointer to the secret and the active mark do not come from outside:
/// the core sets those.
fn profile_from(input: ServerInput, id: String, secret_ref: String) -> ServerProfile {
    let mut p = ServerProfile::new(id, input.name);
    p.host = input.host;
    p.port = input.port;
    p.user = input.user;
    p.auth_kind = input.auth_kind;
    p.key_path = input.key_path;
    p.domain = input.domain;
    p.video_dir = input
        .video_dir
        .filter(|d| !d.trim().is_empty())
        .unwrap_or_else(|| String::from(crate::domain::server_profile::DEFAULT_VIDEO_DIR));
    p.cdn_base = input.cdn_base;
    p.ipv6_mode = input.ipv6_mode;
    p.secret_ref = secret_ref;
    p
}

/// Check a profile and turn the objections into a contract error.
///
/// The objections are joined into one message rather than lost: there are often several,
/// and a person needs to see them all at once rather than one per round.
fn check(profile: &ServerProfile) -> Result<()> {
    profile.validate().map_err(|problems| {
        AppError::new(ErrorCode::InvalidInput)
            .with_details(problems.iter().map(|p| p.detail.clone()))
            // The fields are named in the particulars: the interface highlights them,
            // and a support log should say which ones were wrong without the wording.
            .with_cause(
                problems
                    .iter()
                    .map(|p| p.field)
                    .collect::<Vec<_>>()
                    .join(", "),
            )
    })
}

/// A refusal for a pointer into nothing. A function of its own so that the wording is one
/// and the same across every command: a person should not have to guess whether it is.
pub(crate) fn no_such_server(id: &str) -> AppError {
    AppError::new(ErrorCode::InvalidInput)
        .detail(DetailCode::ProfileNotFound)
        .with_cause(id)
}

pub mod api {
    use super::*;
    use crate::store::profiles;
    use crate::store::secrets::SecretRef;

    /// The list of profiles. Without the secrets — they are physically not here.
    pub fn servers_list(state: &AppState) -> Result<Vec<ServerProfile>> {
        Ok(profiles::list(&state.db)?)
    }

    /// Add a profile. The secret goes to the operating system store; only a pointer is
    /// written into the profile.
    pub fn server_add(state: &AppState, input: ServerInput, secret: &str) -> Result<String> {
        let id = format!("srv_{}", uuid::Uuid::new_v4().simple());
        let reference = SecretRef::for_server(&id);

        let mut profile = profile_from(input, id.clone(), reference.as_str().to_owned());
        profile.normalize();
        check(&profile)?;

        if profiles::name_taken(&state.db, &profile.name, None)? {
            return Err(AppError::new(ErrorCode::InvalidInput)
                .with_detail(
                    Detail::new(DetailCode::ProfileNameTaken).with("name", profile.name.clone()),
                )
                .with_cause(&profile.name));
        }

        profiles::insert(&state.db, &profile)?;

        // The secret goes after the profile is written, and is cleaned up on failure:
        // otherwise an entry nothing points at would be left in the system password
        // manager, and a person would have nothing to delete it with.
        if let Err(e) = state.secrets.set(&reference, secret) {
            let _ = profiles::remove(&state.db, &id);
            return Err(e.into());
        }

        tracing::info!(server = %id, "the server profile was created");
        Ok(id)
    }

    /// Change a profile. The secret is replaced **only when one is passed**: otherwise
    /// changing a profile's name would wipe out the password, and a person would learn of it
    /// only at the next connection.
    pub fn server_update(
        state: &AppState,
        id: &str,
        input: ServerInput,
        secret: Option<&str>,
    ) -> Result<()> {
        let existing = profiles::get(&state.db, id)?.ok_or_else(|| no_such_server(id))?;

        let mut profile = profile_from(input, existing.id.clone(), existing.secret_ref.clone());
        // Editing the fields leaves the active mark and the confirmed fingerprint alone:
        // both of those are deliberate acts of a person's own.
        profile.is_active = existing.is_active;
        profile.host_fingerprint = existing.host_fingerprint.clone();
        profile.normalize();
        check(&profile)?;

        if profiles::name_taken(&state.db, &profile.name, Some(id))? {
            return Err(AppError::new(ErrorCode::InvalidInput)
                .with_detail(
                    Detail::new(DetailCode::ProfileNameTaken).with("name", profile.name.clone()),
                )
                .with_cause(&profile.name));
        }

        profiles::update(&state.db, &profile)?;

        if let Some(value) = secret {
            state
                .secrets
                .set(&SecretRef::from_stored(&profile.secret_ref), value)?;
        }
        Ok(())
    }

    /// Delete a profile along with its secret's entry in the operating system store.
    ///
    /// A secret left behind is access to somebody else's server that a person no longer
    /// remembers (FR-005).
    pub fn server_remove(state: &AppState, id: &str) -> Result<()> {
        // A missing profile is not an error: repeating must be safe (the contract,
        // rule 5).
        let Some(profile) = profiles::get(&state.db, id)? else {
            return Ok(());
        };

        profiles::remove(&state.db, id)?;
        if let Err(e) = state
            .secrets
            .delete(&SecretRef::from_stored(&profile.secret_ref))
        {
            // The profile is already deleted. Reporting the dangling secret matters more
            // than keeping quiet, but an error must not be returned: deleting again would
            // then be impossible, and the profile is already gone.
            tracing::error!(server = %id, error = %e, "the deleted profile's secret stayed in the store");
        }
        tracing::info!(server = %id, "the server profile was deleted");
        Ok(())
    }

    /// Make a profile the active one. Exactly one is active (FR-002).
    pub fn server_set_active(state: &AppState, id: &str) -> Result<()> {
        if profiles::set_active(&state.db, id)? {
            Ok(())
        } else {
            Err(no_such_server(id))
        }
    }

    /// The step-by-step connection check (FR-003).
    ///
    /// It returns **every** step, not only the one that broke: a person needs to see what
    /// managed to pass. The command does not end in an error — a failed step is data rather
    /// than a refusal of the command.
    pub async fn server_test(state: &AppState, id: &str) -> Result<Vec<TestStep>> {
        let profile = profiles::get(&state.db, id)?.ok_or_else(|| no_such_server(id))?;
        Ok(super::probe::run(state, &profile).await)
    }

    /// Offer to carry the settings over from `server.env` (T043).
    ///
    /// A missing file is not an error but an ordinary thing: most people using the
    /// application have none and never will. `None` comes back, and the wizard simply does
    /// not show the offer.
    ///
    /// Nothing is created and nothing is written: this only fills in a form a person will
    /// see and be able to correct.
    pub fn server_import_suggestion(state: &AppState) -> Result<Option<ImportSuggestion>> {
        // The offer makes sense only for a first profile: after that a person sets up
        // their servers themselves, and there is no point filling in the same file again.
        if !profiles::list(&state.db)?.is_empty() {
            return Ok(None);
        }

        Ok(crate::server::env_import::default_location()
            .and_then(|path| crate::server::env_import::read_from(&path))
            .map(|imported| ImportSuggestion {
                source: imported.source.to_string_lossy().into_owned(),
                needs_passphrase: imported.needs_passphrase,
                input: imported.input,
            }))
    }

    /// Confirm a server's fingerprint (FR-092).
    pub fn server_fingerprint_confirm(state: &AppState, id: &str, fingerprint: &str) -> Result<()> {
        let fingerprint = fingerprint.trim();
        if fingerprint.is_empty() {
            return Err(AppError::new(ErrorCode::InvalidInput).detail(DetailCode::FingerprintEmpty));
        }
        if profiles::set_fingerprint(&state.db, id, fingerprint)? {
            tracing::info!(server = %id, "the server's fingerprint was confirmed by the person");
            Ok(())
        } else {
            Err(no_such_server(id))
        }
    }
}

/// The thin wrappers for the shell. There is no logic here — only calls into `api`.
pub mod ipc {
    use super::*;
    use tauri::State;

    #[tauri::command]
    pub fn servers_list(state: State<'_, AppState>) -> Result<Vec<ServerProfile>> {
        api::servers_list(&state)
    }

    #[tauri::command]
    pub fn server_add(
        state: State<'_, AppState>,
        input: ServerInput,
        secret: String,
    ) -> Result<String> {
        api::server_add(&state, input, &secret)
    }

    #[tauri::command]
    pub fn server_update(
        state: State<'_, AppState>,
        id: String,
        input: ServerInput,
        secret: Option<String>,
    ) -> Result<()> {
        api::server_update(&state, &id, input, secret.as_deref())
    }

    #[tauri::command]
    pub fn server_remove(state: State<'_, AppState>, id: String) -> Result<()> {
        api::server_remove(&state, &id)
    }

    #[tauri::command]
    pub fn server_set_active(state: State<'_, AppState>, id: String) -> Result<()> {
        api::server_set_active(&state, &id)
    }

    #[tauri::command]
    pub async fn server_test(state: State<'_, AppState>, id: String) -> Result<Vec<TestStep>> {
        api::server_test(&state, &id).await
    }

    #[tauri::command]
    pub fn server_fingerprint_confirm(
        state: State<'_, AppState>,
        id: String,
        fingerprint: String,
    ) -> Result<()> {
        api::server_fingerprint_confirm(&state, &id, &fingerprint)
    }

    #[tauri::command]
    pub fn server_import_suggestion(
        state: State<'_, AppState>,
    ) -> Result<Option<ImportSuggestion>> {
        api::server_import_suggestion(&state)
    }
}

/// T041 — the step-by-step connection check.
mod probe {
    use super::*;
    use crate::domain::server_profile::AuthKind;
    use crate::ssh::{Connection, Credentials, ServerAddress};
    use crate::store::secrets::SecretRef;
    use std::time::Duration;

    /// How long each step waits for an answer.
    ///
    /// The check runs on a button press, with a person watching it. Half a minute of silence
    /// reads to them as a frozen application rather than a slow server.
    const STEP_TIMEOUT: Duration = Duration::from_secs(10);

    fn step(index: usize, status: StepStatus, detail: Option<Detail>) -> TestStep {
        TestStep {
            id: TEST_STEPS[index].to_owned(),
            status,
            detail,
        }
    }

    /// A step that went well, or did not, with one thing to say about it.
    fn said(index: usize, status: StepStatus, detail: Detail) -> TestStep {
        step(index, status, Some(detail))
    }

    /// A complaint from a library, kept in its own words.
    fn system(e: impl std::fmt::Display) -> Detail {
        Detail::new(DetailCode::SystemError).with("text", crate::store::redact::safe_display(&e))
    }

    /// Fill the remaining steps in as not run.
    ///
    /// This is exactly what separates a check's report from an error message: a person sees
    /// not "something is wrong" but "this passed, it broke off here, and we looked no
    /// further".
    fn skip_rest(steps: &mut Vec<TestStep>) {
        while steps.len() < TEST_STEPS.len() {
            steps.push(step(steps.len(), StepStatus::Skipped, None));
        }
    }

    pub async fn run(state: &AppState, profile: &ServerProfile) -> Vec<TestStep> {
        let mut steps: Vec<TestStep> = Vec::with_capacity(TEST_STEPS.len());

        // 1. The network.
        match reach_ssh(&profile.host, profile.port).await {
            Ok(banner) => steps.push(said(0, StepStatus::Ok, banner)),
            Err(detail) => {
                steps.push(said(0, StepStatus::Failed, detail));
                skip_rest(&mut steps);
                return steps;
            }
        }

        // 2. Logging in. Credentials are never sent to a server whose fingerprint has not
        // been confirmed — so an unconfirmed fingerprint stops the check here rather than
        // turning into a mysterious login failure.
        let Some(expected) = profile.host_fingerprint.clone() else {
            steps.push(said(
                1,
                StepStatus::Failed,
                Detail::new(DetailCode::StepLoginFingerprintUnconfirmed),
            ));
            skip_rest(&mut steps);
            return steps;
        };

        let secret = match state
            .secrets
            .get(&SecretRef::from_stored(&profile.secret_ref))
        {
            Ok(s) => s,
            Err(e) => {
                steps.push(said(1, StepStatus::Failed, system(e)));
                skip_rest(&mut steps);
                return steps;
            }
        };

        let credentials = match profile.auth_kind {
            AuthKind::Key => Credentials::Key {
                path: profile.key_path.clone().unwrap_or_default().into(),
                passphrase: Some(secret),
            },
            AuthKind::Password => Credentials::Password(secret),
        };

        let conn = match Connection::connect(
            ServerAddress::new(&profile.host, profile.port),
            &profile.user,
            credentials,
            &expected,
        )
        .await
        {
            Ok(c) => {
                steps.push(said(
                    1,
                    StepStatus::Ok,
                    Detail::new(DetailCode::StepLoginOk).with("user", profile.user.clone()),
                ));
                c
            }
            Err(e) => {
                // The detail goes through secret redaction: it comes from somebody else's
                // library, which knows nothing of our rules.
                steps.push(said(1, StepStatus::Failed, system(&e)));
                skip_rest(&mut steps);
                return steps;
            }
        };

        // 3. The video directory. Both reading and writing are checked: learning about
        // missing permissions at the first upload is too late — the file is already on its
        // way by then.
        let probe_cmd = format!(
            "test -d '{dir}' && test -r '{dir}' && test -w '{dir}'",
            dir = profile.video_dir
        );
        match conn.exec(&probe_cmd).await {
            Ok(out) if out.ok() => steps.push(said(
                2,
                StepStatus::Ok,
                Detail::new(DetailCode::StepVideoDirOk).with("dir", profile.video_dir.clone()),
            )),
            Ok(_) => {
                steps.push(said(
                    2,
                    StepStatus::Failed,
                    Detail::new(DetailCode::StepVideoDirMissingOrDenied)
                        .with("dir", profile.video_dir.clone())
                        .with("user", profile.user.clone()),
                ));
                skip_rest(&mut steps);
                conn.close().await;
                return steps;
            }
            Err(e) => {
                steps.push(said(2, StepStatus::Failed, system(&e)));
                skip_rest(&mut steps);
                conn.close().await;
                return steps;
            }
        }

        // The name of a real file from the directory is taken — that is what serving will
        // be checked with. This is the difference between "the web server answers" and
        // "the serving works".
        let sample = sample_file(&conn, &profile.video_dir).await;
        conn.close().await;

        // 4. Serving over the domain.
        steps.push(check_domain(&profile.domain, sample.as_deref()).await);
        steps
    }

    /// The name of any video file from the serving directory.
    ///
    /// Needed so that serving is checked with a real file rather than with the directory
    /// root. Having no files is no disaster: a fresh server has none, and the check then
    /// does what it can and says honestly what it did not check.
    async fn sample_file(conn: &Connection, video_dir: &str) -> Option<String> {
        let cmd = format!(
            "find {} -maxdepth 1 -type f -name '*.mp4' -printf '%f\\n' 2>/dev/null | head -n 1",
            crate::server::shell_quote(video_dir)
        );
        let out = conn.exec(&cmd).await.ok()?;
        let name = out.trimmed().trim().to_owned();
        if name.is_empty() {
            None
        } else {
            Some(name)
        }
    }

    /// Reach SSH and make sure it is **really it** there.
    ///
    /// An established connection is not enough. Some hosting providers put attack protection
    /// in front of a server, and it completes the TCP handshake itself on **any** port — and
    /// says nothing. Checked against the author's live server on 2026-08-25: ports 64999,
    /// 12345 and 54321 "answered" exactly as 22 did, with nothing behind them.
    ///
    /// So the step counts as passed only when the server introduced itself: a real SSH sends
    /// an `SSH-2.0-…` line right after connecting, waiting for nothing. It is the same
    /// principle as in checking serving over a domain (R-20): an open port proves nothing,
    /// an answer does.
    async fn reach_ssh(host: &str, port: u16) -> std::result::Result<Detail, Detail> {
        use tokio::io::AsyncReadExt;

        let addr = format!("{host}:{port}");
        let mut stream =
            match tokio::time::timeout(STEP_TIMEOUT, tokio::net::TcpStream::connect(&addr)).await {
                Ok(Ok(s)) => s,
                Ok(Err(e)) => return Err(system(e)),
                Err(_) => {
                    return Err(Detail::new(DetailCode::StepNetTimeout)
                        .with("seconds", STEP_TIMEOUT.as_secs()))
                }
            };

        // The first few dozen bytes are enough for the banner; there is no point waiting
        // long for it — a real SSH sends it immediately.
        let mut buf = [0u8; 128];
        let read = tokio::time::timeout(STEP_TIMEOUT, stream.read(&mut buf)).await;

        let bytes = match read {
            Ok(Ok(0)) => return Err(Detail::new(DetailCode::StepNetClosed)),
            Ok(Ok(n)) => &buf[..n],
            Ok(Err(e)) => return Err(system(e)),
            Err(_) => return Err(Detail::new(DetailCode::StepNetSilent)),
        };

        let banner = String::from_utf8_lossy(bytes);
        let first_line = banner.lines().next().unwrap_or("").trim();
        if first_line.starts_with("SSH-") {
            Ok(Detail::new(DetailCode::StepNetBanner).with("banner", first_line.to_owned()))
        } else {
            Err(Detail::new(DetailCode::StepNetNotSsh)
                .with("port", port)
                .with("got", first_line.chars().take(40).collect::<String>()))
        }
    }

    /// Check that the serving answers over the domain — **from the person's own machine**.
    ///
    /// Checking from inside the server is pointless: from there "it works" even when the
    /// outside cannot reach it because of a domain record or a network filter.
    ///
    /// What this step proves when the directory holds at least one file: the domain
    /// resolves, leads here, the certificate is valid, **and the serving really does hand
    /// out that file's contents**. For the last of those exactly one byte of a real file is
    /// requested: without it the check would come down to "the web server answers", which,
    /// like an open port, proves nothing (R-20).
    ///
    /// When there are no files, only the domain's reachability is checked — and the step's
    /// detail says so, lest the success look fuller than it is.
    async fn check_domain(domain: &str, sample: Option<&str>) -> TestStep {
        let client = match reqwest::Client::builder().timeout(STEP_TIMEOUT).build() {
            Ok(c) => c,
            Err(e) => return said(3, StepStatus::Failed, system(e)),
        };

        let (url, checking_file) = match sample {
            // The file name goes through the same encoding as the viewers' links:
            // otherwise a space or a non-Latin character in a name breaks the check where
            // the serving itself works.
            Some(name) => (
                crate::domain::links::for_path(domain, None, name).origin,
                true,
            ),
            None => (
                format!("https://{domain}/{}/", crate::domain::links::VIDEOS_PREFIX),
                false,
            ),
        };

        // One byte is asked for: that is enough to be sure of the serving, and it does not
        // pull gigabytes off the server for a check.
        let request = if checking_file {
            client.get(&url).header("Range", "bytes=0-0")
        } else {
            client.get(&url)
        };

        match request.send().await {
            Ok(response) => {
                let code = response.status().as_u16();
                if !checking_file {
                    // The web server's answer is a success, but not a full one: the
                    // directory may be closed to listing, and that is the right setting.
                    return said(
                        3,
                        StepStatus::Ok,
                        Detail::new(DetailCode::StepDomainOkNoFiles)
                            .with("domain", domain.to_owned())
                            .with("code", code),
                    );
                }
                if !response.status().is_success() {
                    return said(
                        3,
                        StepStatus::Failed,
                        Detail::new(DetailCode::StepDomainFileNotServed)
                            .with("url", url.clone())
                            .with("code", code),
                    );
                }
                match response.bytes().await {
                    Ok(body) if !body.is_empty() => said(
                        3,
                        StepStatus::Ok,
                        Detail::new(DetailCode::StepDomainOk).with("url", url.clone()),
                    ),
                    Ok(_) => said(
                        3,
                        StepStatus::Failed,
                        Detail::new(DetailCode::StepDomainEmptyBody)
                            .with("url", url.clone())
                            .with("code", code),
                    ),
                    Err(e) => said(3, StepStatus::Failed, system(&e)),
                }
            }
            Err(e) => {
                let detail = if e.is_timeout() {
                    Detail::new(DetailCode::StepDomainTimeout)
                        .with("domain", domain.to_owned())
                        .with("seconds", STEP_TIMEOUT.as_secs())
                } else if e.is_connect() {
                    Detail::new(DetailCode::StepDomainNoConnection)
                        .with("domain", domain.to_owned())
                } else {
                    system(&e)
                };
                said(3, StepStatus::Failed, detail)
            }
        }
    }
}
