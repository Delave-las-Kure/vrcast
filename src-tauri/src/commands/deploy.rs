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
        // The machine is asked before the domain is judged: whether it has an IPv6
        // address of its own is what the IPv6 half of the rule turns on, and the profile
        // cannot say.
        let opened = gate::open(state.secrets.as_ref(), &profile, Intent::Read).await?;
        let facts = machine::look(&opened.conn).await?;
        opened.conn.close().await;
        Ok(look_at_domain(&profile, &facts, ipv6).await)
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
        let domain = look_at_domain(&profile, &facts, ipv6).await;

        // **What a plan may claim about the two proofs.**
        //
        // The key: we are signed in with it at this very moment, so on a key profile it
        // plainly works — said as `true` because it is true, not for convenience. On a
        // password profile there is no key yet and the answer is no.
        //
        // The password: **asked**, and that took a real server to see. Left unasked, the answer
        // is "not established", the hardening step reads as still to do, and a plan on a
        // perfectly hardened server says there is work waiting — for ever. Asking costs one
        // connection that is meant to be refused, which is a read of the server and nothing
        // more.
        let signed_in_by_key = profile.auth_kind != AuthKind::Password;
        let key_now = || -> futures::future::BoxFuture<'_, bool> {
            Box::pin(async move { signed_in_by_key })
        };
        let where_it_is = ServerAddress::new(&profile.host, profile.port);
        let as_whom = profile.user.clone();
        let password_now = move || -> futures::future::BoxFuture<'_, bool> {
            let where_it_is = where_it_is.clone();
            let as_whom = as_whom.clone();
            Box::pin(async move { passwords_are_off(where_it_is, as_whom).await })
        };
        let ctx = Context {
            conn: &opened.conn,
            domain: &profile.domain,
            video_dir: &profile.video_dir,
            ipv6,
            server: addresses_of(&profile, &facts),
            public_key,
            machine: facts.clone(),
            already_ours: opened.state.kind == crate::domain::server_state::Kind::Managed,
            // Planning changes nothing and proves nothing: the two proofs open connections,
            // and a plan that logged in twice per step would be a plan nobody dared ask for.
            proofs: Proofs {
                key_works: &key_now,
                password_refused: &password_now,
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

        // **What a plan may claim about the two proofs.**
        //
        // The key: we are signed in with it at this very moment, so on a key profile it
        // plainly works — said as `true` because it is true, not for convenience. On a
        // password profile there is no key yet and the answer is no.
        //
        // The password: **asked**, and that took a real server to see. Left unasked, the answer
        // is "not established", the hardening step reads as still to do, and a plan on a
        // perfectly hardened server says there is work waiting — for ever. Asking costs one
        // connection that is meant to be refused, which is a read of the server and nothing
        // more.
        let signed_in_by_key = profile.auth_kind != AuthKind::Password;
        let key_now = || -> futures::future::BoxFuture<'_, bool> {
            Box::pin(async move { signed_in_by_key })
        };
        let where_it_is = ServerAddress::new(&profile.host, profile.port);
        let as_whom = profile.user.clone();
        let password_now = move || -> futures::future::BoxFuture<'_, bool> {
            let where_it_is = where_it_is.clone();
            let as_whom = as_whom.clone();
            Box::pin(async move { passwords_are_off(where_it_is, as_whom).await })
        };
        let ctx = Context {
            conn: &opened.conn,
            domain: &profile.domain,
            video_dir: &profile.video_dir,
            ipv6: Ipv6Choice::Keep,
            server: addresses_of(&profile, &facts),
            public_key,
            machine: facts,
            already_ours: true,
            proofs: Proofs {
                key_works: &key_now,
                password_refused: &password_now,
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
            server: addresses_of(&profile, &facts),
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
async fn look_at_domain(
    profile: &ServerProfile,
    machine: &Machine,
    ipv6: Ipv6Choice,
) -> DomainAnswer {
    // A lookup that could not be made at all is not a domain that is not attached, and must
    // not become one: the person would be sent to edit a record that was never wrong.
    let records = dns::look_up(&profile.domain, dns::DEFAULT_PATIENCE)
        .await
        .unwrap_or_else(|_| Records::default());
    let server = addresses_of(profile, machine);
    let verdict = dns_verdict::judge(&records, &server, ipv6);
    DomainAnswer {
        advice: verdict.what_to_do(&profile.domain, &server),
        a: records.a.iter().map(|a| a.to_string()).collect(),
        aaaa: records.aaaa.iter().map(|a| a.to_string()).collect(),
        verdict,
    }
}

/// Where this server is, **as the server itself knows** (T332).
///
/// Not the address in the profile. That one is how we reach the machine; a machine
/// reached over IPv4 very often has an IPv6 address as well, and whether it has one is
/// the whole of the rule about keeping or turning IPv6 off (FR-137).
///
/// Found on the real stand: fed the connection address, the IPv6 half of that rule
/// passed silently on every server reached over IPv4 — which is every server. It did not
/// fail; it agreed.
///
/// The profile's address is the fallback, for the moment before anything has been asked.
fn addresses_of(profile: &ServerProfile, machine: &Machine) -> ServerAddresses {
    let from_profile = profile.host.parse::<std::net::IpAddr>().ok();
    ServerAddresses {
        v4: machine.ipv4().or(match from_profile {
            Some(std::net::IpAddr::V4(v4)) => Some(v4),
            _ => None,
        }),
        v6: machine.ipv6().or(match from_profile {
            Some(std::net::IpAddr::V6(v6)) => Some(v6),
            _ => None,
        }),
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
    if profile.auth_kind == AuthKind::ManagedKey {
        let openssh = secrets
            .get(&SecretRef::from_stored(&profile.secret_ref))
            .map_err(|e| {
                AppError::new(ErrorCode::KeyUnreadable)
                    .with_cause(crate::store::redact::safe_display(&e))
            })?;
        let key = crate::ssh::auth::load_key_text(&openssh, None)?;
        return key
            .public_key()
            .to_openssh()
            .map_err(|e| AppError::new(ErrorCode::KeyUnreadable).with_cause(e));
    }
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
    // The **path**, not the reference to the store. `secret_ref` names an entry holding this
    // key's passphrase; the file itself is `key_path`. Reading one for the other looks right
    // and fails with "no such path" — caught on the real stand, because the container checks
    // build their own context and never come through here.
    let Some(path) = profile.key_path.as_deref().filter(|p| !p.is_empty()) else {
        return Err(AppError::new(ErrorCode::InvalidInput)
            .with_cause("the profile says it signs in with a key and names no key file"));
    };
    let key = crate::ssh::auth::load_key(
        std::path::Path::new(path),
        Some(secret.as_str()).filter(|s| !s.is_empty()),
    )?;
    key.public_key()
        .to_openssh()
        .map_err(|e| AppError::new(ErrorCode::KeyUnreadable).with_cause(e))
}

/// Make a key for a server that is reached by password, and keep it (T290a).
///
/// The private half goes into the operating system's store under this server's own
/// reference — the same place the password was, and it **replaces** it: two ways in kept
/// side by side would mean the password living on in the store after the server had
/// stopped accepting it, which is a secret that is no longer good for anything and can
/// still leak.
///
/// The profile is not switched here. It is switched when the key is proved to work, and
/// until then the password is what gets us in.
fn make_key_for(profile: &ServerProfile) -> Result<crate::ssh::keygen::MadeKey> {
    Ok(crate::ssh::keygen::make(&format!(
        "vrcast-studio: {}",
        profile.name
    ))?)
}

/// Put the made key in the store and point the profile at it.
///
/// Called the moment the `ssh-key` step is known to have worked — **not** when the whole
/// run ends. A deployment that fails after the hardening step leaves a server whose
/// password no longer works; a profile still saying "password" would then be a person
/// locked out of their own machine by a half-finished run.
fn switch_to_managed_key(
    state: &super::AppState,
    profile: &ServerProfile,
    private_openssh: &str,
) -> Result<()> {
    let reference = SecretRef::from_stored(&profile.secret_ref);
    state
        .secrets
        .set(&reference, private_openssh)
        .map_err(|e| {
            AppError::new(ErrorCode::KeyUnreadable)
                .with_cause(crate::store::redact::safe_display(&e))
        })?;

    let switched = ServerProfile {
        auth_kind: AuthKind::ManagedKey,
        // No file was made, so no path may be left behind: a leftover path is the sort of
        // thing that quietly gets used one day.
        key_path: None,
        ..profile.clone()
    };
    crate::store::profiles::update(&state.db, &switched)?;
    Ok(())
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
    // Asked before the connection is let go: the domain cannot be judged without knowing
    // whether this machine has an IPv6 address of its own (T332).
    let facts = machine::look(&opened.conn).await?;
    opened.conn.close().await;

    // **A server reached by password gets a key made for it** (T290a). It has to exist
    // before the hardening step, or that step turns off the only way in — and the ordinary
    // first contact with a bought server is exactly an address and a root password.
    let made = if profile.auth_kind == AuthKind::Password {
        Some(make_key_for(&profile)?)
    } else {
        None
    };
    let public_key = match &made {
        Some(made) => made.public_openssh.clone(),
        None => public_key_for(state.secrets.as_ref(), &profile)?,
    };

    let domain = look_at_domain(&profile, &facts, ipv6).await;
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
    let inner = state.clone();
    let made_private = made.as_ref().map(|m| m.private_openssh.clone());
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
            // What the proof signs in with. For a key we just made, the key itself: the
            // profile still says "password" at this point, and asking it would prove the
            // password works — which is not the question.
            let made_private = made_private.clone();
            let key_path = profile.key_path.clone().map(std::path::PathBuf::from);
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
                let made_private = made_private.clone();
                move || -> futures::future::BoxFuture<'_, bool> {
                    let address = address.clone();
                    let user = user.clone();
                    let key_path = key_path.clone();
                    let passphrase = passphrase.clone();
                    let made_private = made_private.clone();
                    Box::pin(async move {
                        let credentials = match (&made_private, &key_path) {
                            (Some(openssh), _) => Credentials::KeyText {
                                openssh: openssh.clone(),
                                passphrase: None,
                            },
                            (None, Some(path)) => Credentials::Key {
                                path: path.clone(),
                                passphrase,
                            },
                            // Nothing to sign in with. Said as "no" rather than as a
                            // failure: the step above it refuses on the same footing, and
                            // there it can say why.
                            (None, None) => return false,
                        };
                        let Ok(fp) = fingerprint::probe(&address).await else {
                            return false;
                        };
                        Connection::connect(address, user, credentials, &fp)
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
                server: addresses_of(&profile, &facts),
                public_key,
                machine: facts,
                already_ours: kind == crate::tasks::deploy::Kind::Upgrade,
                proofs: Proofs {
                    key_works: &key_works,
                    password_refused: &password_refused,
                },
            };

            let steps = deploy::all();
            // Kept as well as sent: the profile has to be switched the moment the key is
            // known to be in, and that is known from the steps rather than from the run's
            // outcome — a run that failed later still put the key there.
            let mut seen: Vec<PlannedStep> = Vec::new();
            let mut report = |settled: &[PlannedStep]| {
                seen = settled.to_vec();
                let _ = events.send(super::AppEvent::DeployProgress {
                    server_id: server_id.clone(),
                    steps: settled.to_vec(),
                });
            };
            let outcome = crate::tasks::deploy::run(&ctx, &steps, kind, &task, &mut report).await;
            opened.conn.close().await;

            if let Some(private) = &made_private {
                let key_is_in = seen.iter().any(|s| {
                    s.id == crate::domain::deploy_steps::StepId::SshKey
                        && matches!(s.status, crate::domain::deploy_steps::Status::Applied)
                });
                if key_is_in {
                    switch_to_managed_key(&inner, &profile, private)?;
                }
            }
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
