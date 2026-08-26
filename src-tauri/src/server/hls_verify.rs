//! T197 — checking that every variant is really served (FR-047).
//!
//! **Every one, and not only the first.** A ladder whose top rung plays and whose bottom
//! one does not is worse than no ladder: the viewer it was built for — the one on a thin
//! line — is exactly the one who gets nothing. This project has already shipped a ladder
//! with a half-empty variant nobody noticed, because the check stopped at the first.
//!
//! The checking is done from **here**, over the same address a viewer would use, rather
//! than from the server with `curl`. A file can exist on disk, be readable by the serving
//! user, and still not be served — a rule in the wrong place, a stale configuration, a
//! certificate that does not cover the name. Asking from the server's own side proves the
//! file exists; asking from outside proves the thing a person actually needs.

use std::time::Duration;

use serde::{Deserialize, Serialize};

/// How long any one request may take.
///
/// Generous rather than tight: a first request can wait on a certificate being issued, and
/// a check that fails on a slow answer teaches people to run it again rather than to read
/// it.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);

/// What was found about one variant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VariantVerdict {
    /// The variant's own directory, as named in the master.
    pub sub: String,
    /// The playlist answered with a status in the two hundreds.
    pub playlist_served: bool,
    /// How many segments its playlist names.
    pub segments: usize,
    /// The playlist says where it ends.
    ///
    /// **Not a formality.** Without `EXT-X-ENDLIST` a player treats the variant as a live
    /// stream that has not finished yet: it waits for more, and a viewer sees it stall at
    /// the end instead of stopping.
    pub complete: bool,
    /// The first segment could actually be fetched.
    pub first_segment_served: bool,
    /// For fragmented MP4 only: the initialisation piece is served.
    ///
    /// Segments without it are useless — they carry no stream headers at all — and a
    /// playlist can name it correctly while the file itself is missing.
    pub init_served: Option<bool>,
    /// What went wrong, in words, when something did.
    pub trouble: Option<String>,
}

impl VariantVerdict {
    /// Whether this variant is fit to be offered to anybody.
    pub fn ok(&self) -> bool {
        self.playlist_served
            && self.segments > 0
            && self.complete
            && self.first_segment_served
            && self.init_served.unwrap_or(true)
            && self.trouble.is_none()
    }
}

/// What was found about the whole ladder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LadderVerdict {
    pub master_served: bool,
    /// How many variants the master names.
    pub variants_in_master: usize,
    /// How many were expected to be there.
    pub variants_expected: usize,
    pub variants: Vec<VariantVerdict>,
}

impl LadderVerdict {
    /// Success, and only when **every** variant answered.
    pub fn ok(&self) -> bool {
        self.master_served
            && self.variants_expected > 0
            && self.variants_in_master == self.variants_expected
            && !self.variants.is_empty()
            && self.variants.iter().all(|v| v.ok())
    }

    /// The variants that are not fit, by name — what a person is shown when this fails.
    pub fn broken(&self) -> Vec<String> {
        self.variants
            .iter()
            .filter(|v| !v.ok())
            .map(|v| v.sub.clone())
            .collect()
    }
}

/// Ask the serving for every variant of a ladder.
///
/// `master_url` is the address of the master playlist as a viewer would open it.
pub async fn verify(master_url: &str, expected: usize) -> Result<LadderVerdict, VerifyError> {
    let client = reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|e| VerifyError::Unreachable(e.to_string()))?;

    let master = client
        .get(master_url)
        .send()
        .await
        .map_err(|e| VerifyError::Unreachable(e.to_string()))?;
    let master_served = master.status().is_success();
    let master_text = master.text().await.unwrap_or_default();

    let base = master_url.rsplit_once('/').map(|(b, _)| b).unwrap_or("");
    let playlists = crate::domain::hls_master::playlist_paths(&master_text);

    let mut variants = Vec::new();
    for path in &playlists {
        variants.push(check_variant(&client, base, path).await);
    }

    Ok(LadderVerdict {
        master_served,
        variants_in_master: playlists.len(),
        variants_expected: expected,
        variants,
    })
}

async fn check_variant(client: &reqwest::Client, base: &str, path: &str) -> VariantVerdict {
    let sub = path
        .rsplit_once('/')
        .map(|(dir, _)| dir.to_owned())
        .unwrap_or_else(|| path.to_owned());
    let url = format!("{base}/{path}");
    let dir = url
        .rsplit_once('/')
        .map(|(d, _)| d.to_owned())
        .unwrap_or_default();

    let mut verdict = VariantVerdict {
        sub,
        playlist_served: false,
        segments: 0,
        complete: false,
        first_segment_served: false,
        init_served: None,
        trouble: None,
    };

    let text = match client.get(&url).send().await {
        Ok(answer) => {
            verdict.playlist_served = answer.status().is_success();
            if !verdict.playlist_served {
                verdict.trouble = Some(format!("the playlist answered {}", answer.status()));
            }
            answer.text().await.unwrap_or_default()
        }
        Err(e) => {
            verdict.trouble = Some(e.to_string());
            return verdict;
        }
    };

    let segments = crate::domain::hls_master::segment_names(&text);
    verdict.segments = segments.len();
    verdict.complete = text.contains("#EXT-X-ENDLIST");

    if let Some(first) = segments.first() {
        verdict.first_segment_served = fetched(client, &format!("{dir}/{first}")).await;
        if !verdict.first_segment_served {
            verdict.trouble = Some(format!("the first segment, {first}, is not served"));
        }
        // Fragmented MP4 needs its initialisation piece, and a playlist can name it
        // correctly while the file is not there at all.
        if first.ends_with(".m4s") {
            let init = crate::domain::hls_master::init_name(&text)
                .unwrap_or_else(|| String::from("init.mp4"));
            let served = fetched(client, &format!("{dir}/{init}")).await;
            verdict.init_served = Some(served);
            if !served {
                verdict.trouble =
                    Some(format!("{init} is not served, so the segments are useless"));
            }
        }
    } else {
        verdict.trouble = Some(String::from("the playlist names no segments at all"));
    }

    verdict
}

/// Ask for the first few bytes of something and see whether they come.
///
/// A range rather than the whole thing: a segment is megabytes, there is no need to pull
/// one to learn that it is served, and pulling every first segment of every variant would
/// make checking a ladder cost as much as watching it.
async fn fetched(client: &reqwest::Client, url: &str) -> bool {
    match client.get(url).header("Range", "bytes=0-64").send().await {
        Ok(answer) => answer.status().is_success(),
        Err(_) => false,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum VerifyError {
    #[error("the serving could not be reached at all: {0}")]
    Unreachable(String),
}
