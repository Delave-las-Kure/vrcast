//! T287, T288 — the commands that set a server up and bring it up to date.
//!
//! Everything long runs as a task (FR-080) and everything that can be refused quickly is
//! refused before a task exists — above all a domain that does not point here, because a
//! deployment begun on a wrong record costs a half-configured server and a person who cannot
//! tell which half.
//!
//! Nothing here decides whether the application may touch this server. That is the gate's
//! (`server::gate`), and going round it is the one mistake this layer could make that would
//! look like a working program.

use serde::{Deserialize, Serialize};

use crate::domain::deploy_steps::PlannedStep;
use crate::domain::dns_verdict::{self, Ipv6Choice, Records, ServerAddresses, Verdict};
use crate::domain::server_profile::{AuthKind, ServerProfile};
use crate::domain::server_state::ServerState;
use crate::domain::wording::Detail;
use crate::net::dns;
use crate::server::deploy::{self, machine, Context, Machine, Proofs};
use crate::server::gate::{self, Intent};
use crate::server::upgrade;
use crate::ssh::{fingerprint, Connection, Credentials, ServerAddress};
use crate::store::secrets::{SecretRef, SecretStore};

use super::error::{AppError, ErrorCode, Result};

/// What the domain check came back with (FR-137, FR-140).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomainAnswer {
    pub verdict: Verdict,
    /// The addresses found, as text — what a person compares against their registrar's page.
    pub a: Vec<String>,
    pub aaaa: Vec<String>,
    /// What to go and do, when there is something. A code with values; the wording lives in
    /// the interface's dictionaries.
    pub advice: Option<Detail>,
}

impl DomainAnswer {
    pub fn ok(&self) -> bool {
        self.verdict.may_begin()
    }
}

/// What a deployment would do (FR-122).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeployPreview {
    pub domain: DomainAnswer,
    pub steps: Vec<PlannedStep>,
    /// What the machine turned out to be — shown because it decides two of the steps: a small
    /// machine gets a swap file, and a container is told plainly what cannot be done in it.
    pub memory_mb: u32,
    pub disk: String,
}

pub mod api {
    use super::*;

    /// What is this server? (FR-120)
    ///
    /// Read-only, and allowed on anything that answers — including somebody else's machine,
    /// because looking is how a person finds out that it *is* somebody else's.
    pub async fn server_detect(
        state: &super::super::AppState,
        server_id: &str,
    ) -> Result<ServerState> {
        let profile = super::super::library::api::profile_of(state, server_id)?;
        let opened = gate::open(state.secrets.as_ref(), &profile, Intent::Read).await?;
        opened.conn.close().await;
        Ok(opened.state)
    }

    /// Does the domain point here, and does it agree with the choice about IPv6?
    ///
    /// A command of its own so a person can check a record they have just created without
    /// starting a deployment — the change takes minutes to spread, and the answer to "not
    /// yet" is to ask again, not to begin (FR-138).
    pub async fn dns_check(
        state: &super::super::AppState,
        server_id: &str,
        ipv6: Ipv6Choice,
    ) -> Result<DomainAnswer> {
        let profile = super::super::library::api::profile_of(state, server_id)?;
        Ok(look_at_domain(&profile, ipv6).await)
    }

    /// Everything a deployment would change, and nothing changed (FR-122).
    pub async fn deploy_plan(
        state: &super::super::AppState,
        server_id: &str,
        ipv6: Ipv6Choice,
    ) -> Result<DeployPreview> {
        let profile = super::super::library::api::profile_of(state, server_id)?;
        let opened = gate::open(state.secrets.as_ref(), &profile, Intent::Setup).await?;
        let public_key = public_key_for(state.secrets.as_ref(), &profile)?;
        let facts = machine::look(&opened.conn).await?;

        // The domain is asked about here, before the person agrees to anything (FR-137).
        let domain = look_at_domain(&profile, ipv6).await;

        let never = || -> futures::future::BoxFuture<'_, bool> { Box::pin(async { false }) };
        let ctx = Context {
            conn: &opened.conn,
            domain: &profile.domain,
            video_dir: &profile.video_dir,
            ipv6,
            server: addresses_of(&profile).await,
            public_key,
            machine: facts.clone(),
            already_ours: opened.state.kind == crate::domain::server_state::Kind::Managed,
            // Planning changes nothing and proves nothing: the two proofs open connections,
            // and a plan that logged in twice per step would be a plan nobody dared ask for.
            proofs: Proofs {
                key_works: &never,
                password_refused: &never,
            },
        };
        let steps = deploy::all();
        let plan = deploy::plan(&ctx, &steps).await.map_err(step_error)?;
        opened.conn.close().await;

        Ok(DeployPreview {
            domain,
            steps: plan,
            memory_mb: facts.memory_mb,
            disk: facts.disk,
        })
    }

    /// Set the server up. Returns a task number at once (FR-080).
    ///
    /// `confirmed` is not a formality: this installs packages, rewrites the way in and turns
    /// a firewall on, and FR-122 says none of it happens until a person has seen the list and
    /// said yes.
    pub async fn deploy_run(
        state: &super::super::AppState,
        server_id: &str,
        ipv6: Ipv6Choice,
        confirmed: bool,
    ) -> Result<String> {
        if !confirmed {
            return Err(AppError::new(ErrorCode::ConfirmationRequired));
        }
        start(state, server_id, ipv6, crate::tasks::deploy::Kind::Fresh).await
    }

    /// What an upgrade would change (FR-129).
    pub async fn server_upgrade_plan(
        state: &super::super::AppState,
        server_id: &str,
    ) -> Result<upgrade::Plan> {
        let profile = super::super::library::api::profile_of(state, server_id)?;
        let opened = gate::open(state.secrets.as_ref(), &profile, Intent::Read).await?;
        let public_key = public_key_for(state.secrets.as_ref(), &profile)?;
        let facts = machine::look(&opened.conn).await?;
        let from = opened.state.server_version.unwrap_or_default();

        let never = || -> futures::future::BoxFuture<'_, bool> { Box::pin(async { false }) };
        let ctx = Context {
            conn: &opened.conn,
            domain: &profile.domain,
            video_dir: &profile.video_dir,
            ipv6: Ipv6Choice::Keep,
            server: addresses_of(&profile).await,
            public_key,
            machine: facts,
            already_ours: true,
            proofs: Proofs {
                key_works: &never,
                password_refused: &never,
            },
        };
        let steps = deploy::all();
        let plan = upgrade::plan(&ctx, from, &steps)
            .await
            .map_err(step_error)?;
        opened.conn.close().await;
        Ok(plan)
    }

    /// Bring the server side up to date (FR-129, FR-131, FR-133).
    pub async fn server_upgrade_run(
        state: &super::super::AppState,
        server_id: &str,
        confirmed: bool,
    ) -> Result<String> {
        if !confirmed {
            return Err(AppError::new(ErrorCode::ConfirmationRequired));
        }
        start(
            state,
            server_id,
            Ipv6Choice::Keep,
            crate::tasks::deploy::Kind::Upgrade,
        )
        .await
    }

    /// Put the last upgrade's copies back (FR-133).
    ///
    /// Not a task: it copies a handful of small files and reloads two services, and a
    /// progress bar for that would be theatre. It is also the thing somebody reaches for when
    /// an upgrade has just gone wrong, and it should answer at once.
    pub async fn server_rollback(state: &super::super::AppState, server_id: &str) -> Result<()> {
        let profile = super::super::library::api::profile_of(state, server_id)?;
        let opened = gate::open(state.secrets.as_ref(), &profile, Intent::Read).await?;
        let public_key = public_key_for(state.secrets.as_ref(), &profile)?;
        let facts = machine::look(&opened.conn).await?;

        let never = || -> futures::future::BoxFuture<'_, bool> { Box::pin(async { false }) };
        let ctx = Context {
            conn: &opened.conn,
            domain: &profile.domain,
            video_dir: &profile.video_dir,
            ipv6: Ipv6Choice::Keep,
            server: addresses_of(&profile).await,
            public_key,
            machine: facts,
            already_ours: true,
            proofs: Proofs {
                key_works: &never,
                password_refused: &never,
            },
        };
        let outcome = upgrade::roll_back(&ctx).await.map_err(step_error);
        opened.conn.close().await;
        outcome
    }
}

/// Ask the domain and judge it.
async fn look_at_domain(profile: &ServerProfile, ipv6: Ipv6Choice) -> DomainAnswer {
    // A lookup that could not be made at all is not a domain that is not attached, and must
    // not become one: the person would be sent to edit a record that was never wrong.
    let records = dns::look_up(&profile.domain, dns::DEFAULT_PATIENCE)
        .await
        .unwrap_or_else(|_| Records::default());
    let server = addresses_of(profile).await;
    let verdict = dns_verdict::judge(&records, &server, ipv6);
    DomainAnswer {
        advice: verdict.what_to_do(&profile.domain, &server),
        a: records.a.iter().map(|a| a.to_string()).collect(),
        aaaa: records.aaaa.iter().map(|a| a.to_string()).collect(),
        verdict,
    }
}

/// Where this server is.
///
/// The profile's host is what we reach it by, so it is the answer when it is an address. When
/// it is a name, it is looked up — a person may well have entered the same name they are
/// deploying under.
async fn addresses_of(profile: &ServerProfile) -> ServerAddresses {
    if let Ok(one) = profile.host.parse::<std::net::IpAddr>() {
        return match one {
            std::net::IpAddr::V4(v4) => ServerAddresses {
                v4: Some(v4),
                v6: None,
            },
            std::net::IpAddr::V6(v6) => ServerAddresses {
                v4: None,
                v6: Some(v6),
            },
        };
    }
    let found = dns::look_up(&profile.host, std::time::Duration::from_millis(1))
        .await
        .unwrap_or_default();
    ServerAddresses {
        v4: found.a.first().copied(),
        v6: found.aaaa.first().copied(),
    }
}

/// The public half of the key this profile signs in with.
///
/// **A password-only profile cannot be deployed from yet, and it says so.** Turning password
/// logins off without a key first would lock the application — and the person — out, so the
/// step order refuses it (R-12). Making a key of our own would be the way round that, and it
/// is a decision rather than a line of code: a private key has to live somewhere, and this
/// project keeps secrets in the operating system's store rather than in files (principle IV),
/// while the way in takes a **path**. Left as a named gap instead of guessed at.
fn public_key_for(secrets: &dyn SecretStore, profile: &ServerProfile) -> Result<String> {
    if profile.auth_kind != AuthKind::Key {
        return Err(AppError::new(ErrorCode::InvalidInput).with_cause(
            "this server's profile signs in with a password, and a deployment needs a key to \
             put on the server before it turns password logins off",
        ));
    }
    let secret = secrets
        .get(&SecretRef::from_stored(&profile.secret_ref))
        .map_err(|e| {
            AppError::new(ErrorCode::KeyUnreadable)
                .with_cause(crate::store::redact::safe_display(&e))
        })?;
    let key = crate::ssh::auth::load_key(
        std::path::Path::new(&profile.secret_ref),
        Some(secret.as_str()).filter(|s| !s.is_empty()),
    )?;
    key.public_key()
        .to_openssh()
        .map_err(|e| AppError::new(ErrorCode::KeyUnreadable).with_cause(e))
}

/// A failure inside the deployment layer, as a contract code.
fn step_error(e: crate::server::deploy::DeployError) -> AppError {
    use crate::server::deploy::DeployError as E;
    match e {
        E::Ssh(inner) => inner.into(),
        E::Cancelled => AppError::new(ErrorCode::TaskCancelled),
        other => AppError::new(ErrorCode::DeployStepFailed).with_cause(other),
    }
}

/// Start a deployment or an upgrade as a task.
async fn start(
    state: &super::AppState,
    server_id: &str,
    ipv6: Ipv6Choice,
    kind: crate::tasks::deploy::Kind,
) -> Result<String> {
    let profile = super::library::api::profile_of(state, server_id)?;
    let intent = Intent::Setup;

    // Refused before a task exists: the door, the key, and the domain. Each of them costs one
    // question now and hours of somebody's evening later.
    let opened = gate::open(state.secrets.as_ref(), &profile, intent).await?;
    opened.conn.close().await;
    let public_key = public_key_for(state.secrets.as_ref(), &profile)?;

    let domain = look_at_domain(&profile, ipv6).await;
    if !domain.ok() {
        let mut error = AppError::new(match domain.verdict {
            Verdict::Ipv6Mismatch { .. } => ErrorCode::Ipv6Mismatch,
            Verdict::PointsElsewhere { .. } => ErrorCode::DomainPointsElsewhere,
            _ => ErrorCode::DomainNotPointed,
        });
        if let Some(advice) = domain.advice {
            error = error.with_detail(advice);
        }
        return Err(error);
    }

    let secrets = state.secrets.clone();
    let events = state.events.clone();
    let task_kind = match kind {
        crate::tasks::deploy::Kind::Fresh => crate::tasks::state::TaskKind::Deploy,
        crate::tasks::deploy::Kind::Upgrade => crate::tasks::state::TaskKind::UpgradeServer,
    };
    let server_id = server_id.to_owned();

    let task_id = state
        .tasks
        .submit(task_kind, Some(server_id.clone()), move |task| async move {
            let opened = gate::open(secrets.as_ref(), &profile, intent).await?;
            let facts: Machine = machine::look(&opened.conn).await?;
            let address = ServerAddress::new(&profile.host, profile.port);
            let user = profile.user.clone();
            let key_path = std::path::PathBuf::from(&profile.secret_ref);
            let passphrase = secrets
                .get(&SecretRef::from_stored(&profile.secret_ref))
                .ok()
                .filter(|s| !s.is_empty());

            // The two proofs, each on a connection of its own. The one we are holding would
            // go on working whatever we did to the settings — which is exactly what makes it
            // the wrong witness (T274).
            let key_works = {
                let address = address.clone();
                let user = user.clone();
                let key_path = key_path.clone();
                let passphrase = passphrase.clone();
                move || -> futures::future::BoxFuture<'_, bool> {
                    let address = address.clone();
                    let user = user.clone();
                    let key_path = key_path.clone();
                    let passphrase = passphrase.clone();
                    Box::pin(async move {
                        let Ok(fp) = fingerprint::probe(&address).await else {
                            return false;
                        };
                        Connection::connect(
                            address,
                            user,
                            Credentials::Key {
                                path: key_path,
                                passphrase,
                            },
                            &fp,
                        )
                        .await
                        .is_ok()
                    })
                }
            };
            let password_refused = {
                let address = address.clone();
                let user = user.clone();
                move || -> futures::future::BoxFuture<'_, bool> {
                    let address = address.clone();
                    let user = user.clone();
                    Box::pin(async move { passwords_are_off(address, user).await })
                }
            };

            let ctx = Context {
                conn: &opened.conn,
                domain: &profile.domain,
                video_dir: &profile.video_dir,
                ipv6,
                server: addresses_of(&profile).await,
                public_key,
                machine: facts,
                already_ours: kind == crate::tasks::deploy::Kind::Upgrade,
                proofs: Proofs {
                    key_works: &key_works,
                    password_refused: &password_refused,
                },
            };

            let steps = deploy::all();
            let mut report = |settled: &[PlannedStep]| {
                let _ = events.send(super::AppEvent::DeployProgress {
                    server_id: server_id.clone(),
                    steps: settled.to_vec(),
                });
            };
            let outcome = crate::tasks::deploy::run(&ctx, &steps, kind, &task, &mut report).await;
            opened.conn.close().await;
            outcome.map(|_| ())
        })
        .await?;
    Ok(task_id)
}

/// Are password logins really off?
///
/// **Not "did a password fail"** — a wrong password fails too, and the two look the same from
/// outside. What settles it is the list of methods the server names when it turns an attempt
/// down: a server that still allows passwords names `Password` among them.
async fn passwords_are_off(address: ServerAddress, user: String) -> bool {
    use crate::ssh::SshError;

    let Ok(fp) = fingerprint::probe(&address).await else {
        return false;
    };
    // A password that will not be right. What is being read is the refusal, not the attempt.
    match Connection::connect(
        address,
        user,
        Credentials::Password(String::from("vrcast-checking-whether-passwords-are-off")),
        &fp,
    )
    .await
    {
        Ok(conn) => {
            // It let us in with that. Passwords are very much on.
            conn.close().await;
            false
        }
        Err(SshError::AuthFailed { methods }) => !methods.to_lowercase().contains("password"),
        // Anything else is not an answer about passwords.
        Err(_) => false,
    }
}

pub mod ipc {
    use super::*;
    use tauri::State;

    #[tauri::command]
    pub async fn server_detect(
        state: State<'_, super::super::AppState>,
        server_id: String,
    ) -> Result<ServerState> {
        api::server_detect(&state, &server_id).await
    }

    #[tauri::command]
    pub async fn dns_check(
        state: State<'_, super::super::AppState>,
        server_id: String,
        ipv6: Ipv6Choice,
    ) -> Result<DomainAnswer> {
        api::dns_check(&state, &server_id, ipv6).await
    }

    #[tauri::command]
    pub async fn deploy_plan(
        state: State<'_, super::super::AppState>,
        server_id: String,
        ipv6: Ipv6Choice,
    ) -> Result<DeployPreview> {
        api::deploy_plan(&state, &server_id, ipv6).await
    }

    #[tauri::command]
    pub async fn deploy_run(
        state: State<'_, super::super::AppState>,
        server_id: String,
        ipv6: Ipv6Choice,
        confirmed: bool,
    ) -> Result<String> {
        api::deploy_run(&state, &server_id, ipv6, confirmed).await
    }

    #[tauri::command]
    pub async fn server_upgrade_plan(
        state: State<'_, super::super::AppState>,
        server_id: String,
    ) -> Result<upgrade::Plan> {
        api::server_upgrade_plan(&state, &server_id).await
    }

    #[tauri::command]
    pub async fn server_upgrade_run(
        state: State<'_, super::super::AppState>,
        server_id: String,
        confirmed: bool,
    ) -> Result<String> {
        api::server_upgrade_run(&state, &server_id, confirmed).await
    }

    #[tauri::command]
    pub async fn server_rollback(
        state: State<'_, super::super::AppState>,
        server_id: String,
    ) -> Result<()> {
        api::server_rollback(&state, &server_id).await
    }
}
