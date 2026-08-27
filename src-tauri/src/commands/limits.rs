//! T214, T215 — commands for capping a viewer's quality (FR-061…FR-067).
//!
//! Contract: `contracts/ipc-commands.md`, "Зрители и ограничения".
//!
//! **The warnings come before the change, not after it** (FR-066). What is being edited is
//! the configuration of the thing serving somebody's film at that moment, and three of the
//! things worth knowing cannot be undone by knowing them later: that the address may belong
//! to more than one person, that it may stop belonging to this one, and that the cap is
//! below anything that exists.

use serde::{Deserialize, Serialize};

use super::error::{AppError, DetailCode, ErrorCode, Result};
use crate::domain::hls_master::{self, Variant};
use crate::domain::limits_conf::Limit;
use crate::domain::slow_master::shorten;
use crate::domain::wording::Detail;
use crate::server::gate::{self, Intent};
use crate::server::limits::{LimitError, Serving};
use crate::server::shell_quote;

/// Where the media sit in an address on this project's servers.
const SERVING_PREFIX: &str = "/videos";

/// The file this application owns, and the one it only reads.
const LIMITS_CONF: &str = "/etc/caddy/vrcast-limits.conf";
const MAIN_CONF: &str = "/etc/caddy/Caddyfile";

/// What the interface sends to cap somebody.
#[derive(Debug, Clone, Deserialize)]
pub struct LimitRequest {
    pub server_id: String,
    /// The viewer's address **as the serving sees it**.
    ///
    /// Taken from the access log and from nowhere else. A viewer's address over HTTP is not
    /// always their address over anything else — on this project's own server the two
    /// differed, and a rule written for the wrong one looks perfectly correct and does
    /// nothing at all.
    pub ip: String,
    /// The medium's own directory.
    pub slug: String,
    pub cap_bps: u64,
}

/// What a limit would do, said before it is done.
#[derive(Debug, Clone, Serialize)]
pub struct LimitPreview {
    /// The rungs this viewer would be left with, heaviest first.
    pub kept: Vec<Variant>,
    /// Everything a person should know before agreeing.
    pub warnings: Vec<Detail>,
    /// The cap is under the lightest rung there is, so the lightest is given anyway.
    pub below_lightest: bool,
}

pub mod api {
    use super::*;

    /// What capping this viewer would do. Nothing is changed.
    pub async fn limit_preview(
        state: &super::super::AppState,
        request: &LimitRequest,
    ) -> Result<LimitPreview> {
        let profile = super::super::library::api::profile_of(state, &request.server_id)?;
        // Only looking: what the set holds, so a cap can be offered against it.
        let conn = gate::open(state.secrets.as_ref(), &profile, Intent::Read)
            .await?
            .conn;
        let variants = ladder_of(&conn, &profile.video_dir, &request.slug).await?;
        conn.close().await;

        let short = shorten(&variants, request.cap_bps, SERVING_PREFIX, &request.slug);
        let mut warnings = Vec::new();

        // A limit is put on an address, and an address is not a person.
        warnings.push(Detail::new(DetailCode::WarnLimitFollowsTheAddress));

        // Several viewers behind one address is the ordinary case for a household or an
        // office, and this would quietly cap all of them.
        let sharing = state
            .viewers
            .active_now()
            .iter()
            .filter(|v| v.ip == request.ip)
            .count();
        if sharing > 1 {
            warnings.push(Detail::new(DetailCode::WarnAddressShared).with("count", sharing as u64));
        }

        if short.below_lightest {
            warnings.push(Detail::new(DetailCode::WarnCapBelowLightest).with(
                "lightest_bps",
                short.kept.first().map(|v| v.bandwidth).unwrap_or(0),
            ));
        }

        Ok(LimitPreview {
            kept: short.kept,
            warnings,
            below_lightest: short.below_lightest,
        })
    }

    /// Put the cap on.
    ///
    /// Refuses unless the person has seen the warnings and said yes: a change to a serving
    /// configuration made without a word is not a thing this application does.
    pub async fn limit_set(
        state: &super::super::AppState,
        request: LimitRequest,
        confirmed: bool,
    ) -> Result<()> {
        if !confirmed {
            return Err(AppError::new(ErrorCode::ConfirmationRequired)
                .detail(DetailCode::WarnLimitFollowsTheAddress));
        }

        let profile = super::super::library::api::profile_of(state, &request.server_id)?;
        let conn = gate::open(state.secrets.as_ref(), &profile, Intent::Change)
            .await?
            .conn;
        let variants = ladder_of(&conn, &profile.video_dir, &request.slug).await?;
        let short = shorten(&variants, request.cap_bps, SERVING_PREFIX, &request.slug);

        let check_url = crate::domain::links::for_path(
            &profile.domain,
            None,
            &format!("{}/master.m3u8", request.slug),
        )
        .origin;
        let serving = Serving {
            conn: &conn,
            video_dir: &profile.video_dir,
            conf_path: LIMITS_CONF,
            main_conf: MAIN_CONF,
            serving_prefix: SERVING_PREFIX,
            check_url: &check_url,
            owner: &format!("{}:{}", profile.user, profile.user),
        };

        // Everything already in force, with this one replacing any it repeats. The file is
        // written whole, so what is not in this list stops existing.
        let mut limits: Vec<Limit> = serving
            .limits()
            .await
            .map_err(to_error)?
            .into_iter()
            .filter(|l| !(l.ip == request.ip && l.slug == request.slug))
            .collect();
        limits.push(Limit {
            ip: request.ip.clone(),
            slug: request.slug.clone(),
            cap_bps: request.cap_bps,
            set_at: crate::store::db::now_rfc3339(),
        });

        let outcome = serving
            .apply(&limits, &[(request.slug.clone(), short)])
            .await;
        conn.close().await;
        outcome.map_err(to_error)
    }

    /// Take the cap off (FR-065).
    pub async fn limit_clear(
        state: &super::super::AppState,
        server_id: &str,
        ip: &str,
        slug: &str,
    ) -> Result<()> {
        let profile = super::super::library::api::profile_of(state, server_id)?;
        let conn = gate::open(state.secrets.as_ref(), &profile, Intent::Change)
            .await?
            .conn;

        let check_url =
            crate::domain::links::for_path(&profile.domain, None, &format!("{slug}/master.m3u8"))
                .origin;
        let serving = Serving {
            conn: &conn,
            video_dir: &profile.video_dir,
            conf_path: LIMITS_CONF,
            main_conf: MAIN_CONF,
            serving_prefix: SERVING_PREFIX,
            check_url: &check_url,
            owner: &format!("{}:{}", profile.user, profile.user),
        };

        let remaining: Vec<Limit> = serving
            .limits()
            .await
            .map_err(to_error)?
            .into_iter()
            .filter(|l| !(l.ip == ip && l.slug == slug))
            .collect();
        // The shortened description goes only when nothing else still points at it.
        let still_wanted = remaining.iter().any(|l| l.slug == slug);
        let outcome = if still_wanted {
            serving.apply(&remaining, &[]).await
        } else {
            serving.clear(&remaining, slug).await
        };
        conn.close().await;
        outcome.map_err(to_error)
    }

    /// What limits are in force — read from the server (FR-064).
    pub async fn limits_list(
        state: &super::super::AppState,
        server_id: &str,
    ) -> Result<Vec<Limit>> {
        let profile = super::super::library::api::profile_of(state, server_id)?;
        // Only looking: which caps are in force.
        let conn = gate::open(state.secrets.as_ref(), &profile, Intent::Read)
            .await?
            .conn;
        let serving = Serving {
            conn: &conn,
            video_dir: &profile.video_dir,
            conf_path: LIMITS_CONF,
            main_conf: MAIN_CONF,
            serving_prefix: SERVING_PREFIX,
            check_url: "",
            owner: "",
        };
        let found = serving.limits().await.map_err(to_error);
        conn.close().await;
        found
    }
}

/// The quality set a medium has, or a refusal saying it has none.
///
/// **T215.** A file served directly has one quality and there is nothing to shorten: the
/// only honest answer is that this cannot be done here, and the person should be pointed at
/// building a set rather than left wondering why the button did nothing.
async fn ladder_of(
    conn: &crate::ssh::Connection,
    video_dir: &str,
    slug: &str,
) -> Result<Vec<Variant>> {
    let path = format!("{}/{slug}/master.m3u8", video_dir.trim_end_matches('/'));
    let out = conn
        .exec(&format!("cat {} 2>/dev/null || true", shell_quote(&path)))
        .await
        .map_err(|e| AppError::new(ErrorCode::Internal).with_cause(e))?;

    let variants = hls_master::parse(&out.stdout).unwrap_or_default();
    if variants.is_empty() {
        return Err(AppError::new(ErrorCode::NoLadderForMedia).with_cause(slug));
    }
    Ok(variants)
}

fn to_error(e: LimitError) -> AppError {
    match e {
        LimitError::ValidateFailed(said) => {
            AppError::new(ErrorCode::CaddyValidateFailed).with_cause(said)
        }
        LimitError::ReloadFailed(said) => {
            AppError::new(ErrorCode::CaddyReloadFailed).with_cause(said)
        }
        other => AppError::new(ErrorCode::Internal).with_cause(other),
    }
}

pub mod ipc {
    use super::*;
    use tauri::State;

    #[tauri::command]
    pub async fn limit_preview(
        state: State<'_, super::super::AppState>,
        request: LimitRequest,
    ) -> Result<LimitPreview> {
        api::limit_preview(&state, &request).await
    }

    #[tauri::command]
    pub async fn limit_set(
        state: State<'_, super::super::AppState>,
        request: LimitRequest,
        confirmed: bool,
    ) -> Result<()> {
        api::limit_set(&state, request, confirmed).await
    }

    #[tauri::command]
    pub async fn limit_clear(
        state: State<'_, super::super::AppState>,
        server_id: String,
        ip: String,
        slug: String,
    ) -> Result<()> {
        api::limit_clear(&state, &server_id, &ip, &slug).await
    }

    #[tauri::command]
    pub async fn limits_list(
        state: State<'_, super::super::AppState>,
        server_id: String,
    ) -> Result<Vec<Limit>> {
        api::limits_list(&state, &server_id).await
    }
}
