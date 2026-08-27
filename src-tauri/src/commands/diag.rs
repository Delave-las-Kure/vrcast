//! T316 — the four things a person can ask when something is wrong (FR-070 – FR-073).
//!
//! Reading only. Nothing here changes a server, and that is why every one of them is allowed
//! through the gate on `Intent::Read` — including on somebody else's machine and on a server
//! newer than this application, where looking is exactly what a person needs to be able to do.
//!
//! **The order these are meant to be used in is the order they are declared in**, and it is
//! the method the diagnosis skill records: is the server asleep, then what does its log say,
//! then why is this particular viewer struggling, and only then — is it the file. Asking them
//! the other way round is how an afternoon goes into re-encoding a film to fix somebody's
//! Wi-Fi.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::domain::health::{Rated, Rating, Snapshot};
use crate::domain::log_digest::{self, Digest};
use crate::domain::stalls::{self, FileShape, Load, SetAside, Verdict, Watcher};
use crate::media::measure::{self, Peaks};
use crate::server::gate::{self, Intent};
use crate::server::{health, log_digest as log_reader};

use super::error::{AppError, ErrorCode, Result};

/// How the server is (FR-070).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Health {
    /// The readings themselves, so a person can see what the judgement was made of.
    pub snapshot: Snapshot,
    pub readings: Vec<Rated>,
    /// The worst of them, which is what a badge shows.
    pub worst: Rating,
}

/// What a stretch of the log adds up to (FR-071).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Logs {
    pub digest: Digest,
    /// Whether the stretch asked for was longer than what could be brought across. Said out
    /// loud: a digest quietly covering less than it was asked for answers a different
    /// question than the one put to it.
    pub reached_the_cap: bool,
    pub oldest: Option<String>,
}

/// Who was watching, how they were doing, and why (FR-072).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Stalls {
    pub watchers: Vec<Watcher>,
    /// Addresses that were not viewers, and why. Shown rather than swallowed — a person who
    /// knows a cache node is pulling wants to see that it was recognised.
    pub set_aside: Vec<SetAside>,
    /// One conclusion per viewer, in the same order.
    pub verdicts: Vec<Verdict>,
    /// What the server itself was doing while all this happened.
    pub load: Load,
}

/// The longest stretch of log that may be asked for in one go, in minutes.
///
/// **A choice.** Four hours is longer than any watching this application is built around, and
/// beyond it the cap on lines would be doing the deciding instead of the person.
pub const MOST_MINUTES: u32 = 240;

pub mod api {
    use super::*;

    /// How is the server? (FR-070)
    pub async fn diag_health(state: &super::super::AppState, server_id: &str) -> Result<Health> {
        let profile = super::super::library::api::profile_of(state, server_id)?;
        let opened = gate::open(state.secrets.as_ref(), &profile, Intent::Read).await?;
        let snapshot = health::look(&opened.conn, &profile.video_dir, &profile.domain).await;
        opened.conn.close().await;

        let snapshot = snapshot?;
        let readings = crate::domain::health::judge(&snapshot);
        let worst = crate::domain::health::worst(&readings);
        Ok(Health {
            snapshot,
            readings,
            worst,
        })
    }

    /// What has the serving been doing? (FR-071)
    pub async fn diag_logs(
        state: &super::super::AppState,
        server_id: &str,
        minutes: u32,
    ) -> Result<Logs> {
        let minutes = sane_minutes(minutes)?;
        let profile = super::super::library::api::profile_of(state, server_id)?;
        let opened = gate::open(state.secrets.as_ref(), &profile, Intent::Read).await?;
        let stretch = log_reader::over(
            &opened.conn,
            since(minutes),
            None,
            log_reader::ACCESS_LOG_PATH,
        )
        .await;
        opened.conn.close().await;

        let stretch = stretch?;
        Ok(Logs {
            digest: log_digest::digest(&stretch.requests, stretch.unreadable),
            reached_the_cap: stretch.reached_the_cap,
            oldest: stretch.oldest.map(|at| at.to_string()),
        })
    }

    /// Why is the picture stopping? (FR-072)
    ///
    /// `file` is what a `diag_bitrate` found, when one has been run. Absent, the answer simply
    /// never blames the file — which is right: the file is the **last** thing the method looks
    /// at, and blaming it without having measured it would be a guess wearing a conclusion's
    /// clothes.
    pub async fn diag_explain_stalls(
        state: &super::super::AppState,
        server_id: &str,
        minutes: u32,
        file: Option<FileShape>,
    ) -> Result<Stalls> {
        let minutes = sane_minutes(minutes)?;
        let profile = super::super::library::api::profile_of(state, server_id)?;
        let opened = gate::open(state.secrets.as_ref(), &profile, Intent::Read).await?;

        // The live readings first and the log second, on the same connection: what the server
        // was doing has to be measured while the complaint is still happening, and the log is
        // written down and will keep.
        let live = health::load(&opened.conn).await;
        let stretch = log_reader::over(
            &opened.conn,
            since(minutes),
            None,
            log_reader::ACCESS_LOG_PATH,
        )
        .await;
        opened.conn.close().await;

        let live = live?;
        let sifted = stalls::sift(&stretch?.requests, &live.addresses);
        let verdicts = sifted
            .watchers
            .iter()
            .map(|w| stalls::explain(w, Some(&live.load), file.as_ref()))
            .collect();

        Ok(Stalls {
            watchers: sifted.watchers,
            set_aside: sifted.set_aside,
            verdicts,
            load: live.load,
        })
    }

    /// Where does this file peak? (FR-073)
    ///
    /// A local file, and no server is touched: the question is about the film, and the answer
    /// is the same whether it has been uploaded yet or not. Which means it can be asked
    /// **before** an upload, which is when it is most useful.
    pub async fn diag_bitrate(path: &str) -> Result<Peaks> {
        let path = PathBuf::from(path);
        measure::peaks_of(&path)
            .await
            .map_err(|e| AppError::new(ErrorCode::FfmpegBroken).with_cause(e))
    }

    fn since(minutes: u32) -> OffsetDateTime {
        OffsetDateTime::now_utc() - time::Duration::minutes(i64::from(minutes))
    }

    fn sane_minutes(minutes: u32) -> Result<u32> {
        if minutes == 0 || minutes > MOST_MINUTES {
            return Err(AppError::new(ErrorCode::InvalidInput));
        }
        Ok(minutes)
    }
}

pub mod ipc {
    use super::*;
    use tauri::State;

    #[tauri::command]
    pub async fn diag_health(
        state: State<'_, super::super::AppState>,
        server_id: String,
    ) -> Result<Health> {
        api::diag_health(&state, &server_id).await
    }

    #[tauri::command]
    pub async fn diag_logs(
        state: State<'_, super::super::AppState>,
        server_id: String,
        minutes: u32,
    ) -> Result<Logs> {
        api::diag_logs(&state, &server_id, minutes).await
    }

    #[tauri::command]
    pub async fn diag_explain_stalls(
        state: State<'_, super::super::AppState>,
        server_id: String,
        minutes: u32,
        file: Option<FileShape>,
    ) -> Result<Stalls> {
        api::diag_explain_stalls(&state, &server_id, minutes, file).await
    }

    #[tauri::command]
    pub async fn diag_bitrate(path: String) -> Result<Peaks> {
        api::diag_bitrate(&path).await
    }
}
