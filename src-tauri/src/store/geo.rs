//! T162 — the table of places: where a viewer's address is, worked out on this machine.
//!
//! **Why it is fetched rather than put into the installer.** R-08 said "built in", and that
//! was decided before anyone had measured the file: the city table is 62 MB compressed and
//! the provider table another 5. FR-112 requires the application to bring what it needs for
//! **working with video** — and to be allowed to "get it itself, without asking anything of
//! the person". A table of places is not needed for working with video at all: without it
//! the application does everything it does and says "not determined", which is the truth.
//! FFmpeg is the other case entirely — without that, nothing works.
//!
//! **Why no snapshot is pinned.** A pinned month disappears from DB-IP within months, and a
//! build that worked in August fails in December for whoever tries it. Asking for the
//! **current** month instead means the address always exists and no mirror of our own is
//! needed. The price is that what comes back is not verified against a snapshot — far
//! milder here than for FFmpeg: the worst a spoilt table can do is name the wrong city,
//! whereas a spoilt program runs.
//!
//! **Why nothing is sent anywhere.** FR-057. The lookup happens here, on the viewer's
//! owner's machine. Asking an outside service would mean handing it the address of every
//! viewer, every session — and those are somebody's friends, who were not asked.

use std::path::{Path, PathBuf};

use maxminddb::{path, Reader};

use crate::domain::geo::{is_not_public, Place};

/// Where DB-IP publishes the free tables.
const BASE: &str = "https://download.db-ip.com/free";

/// What the two tables are called once they are here.
const CITY: &str = "dbip-city-lite.mmdb";
const ASN: &str = "dbip-asn-lite.mmdb";

/// Beside the file the tables came from, so that "which month is this" needs no guessing.
const STAMP: &str = "dbip-month.txt";

/// The two tables, open and ready to answer.
///
/// Both may be absent, and that is the state the application ships in: everything works and
/// every viewer is "not determined" until the tables arrive.
#[derive(Default)]
pub struct Places {
    city: Option<Reader<Vec<u8>>>,
    asn: Option<Reader<Vec<u8>>>,
}

impl std::fmt::Debug for Places {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Places")
            .field("city", &self.city.is_some())
            .field("asn", &self.asn.is_some())
            .finish()
    }
}

impl Places {
    /// Open whatever is on disk. Nothing there is normal rather than an error.
    pub fn open(dir: &Path) -> Self {
        Self {
            city: open_one(&dir.join(CITY)),
            asn: open_one(&dir.join(ASN)),
        }
    }

    /// Whether anything at all can be answered.
    pub fn is_empty(&self) -> bool {
        self.city.is_none() && self.asn.is_none()
    }

    /// Where an address is.
    ///
    /// An empty answer means **not determined**, and it is shown as that. Nothing here is
    /// ever guessed at from a neighbouring range (FR-052).
    pub fn look_up(&self, ip: &str) -> Place {
        let Ok(address) = ip.parse::<std::net::IpAddr>() else {
            return Place::default();
        };
        // No table can speak for a private or a loopback address, and one that answers is
        // answering about its own reserved rows. Somebody watching from the next room is
        // "not determined", not whatever happens to sit there.
        if is_not_public(&address) {
            return Place::default();
        }

        let mut place = Place::default();
        if let Some(city) = &self.city {
            if let Ok(found) = city.lookup(address) {
                // The named country if the table carries names, and the two-letter code if
                // it does not. A code is a poor answer but an honest one — better than an
                // empty cell, which reads as a fault in the application rather than as
                // something the table does not know.
                place.country = text(found.decode_path(&path!["country", "names", "en"]))
                    .or_else(|| text(found.decode_path(&path!["country", "iso_code"])));
                place.city = text(found.decode_path(&path!["city", "names", "en"]));
            }
        }
        if let Some(asn) = &self.asn {
            if let Ok(found) = asn.lookup(address) {
                place.asn_org = text(found.decode_path(&path!["autonomous_system_organization"]));
            }
        }
        place
    }
}

/// What the reader gives back, when it is a piece of text worth showing.
///
/// A field that is absent, unreadable or empty all come to the same thing here: not
/// determined. Telling them apart would give the person nothing to act on.
fn text(decoded: Result<Option<String>, maxminddb::MaxMindDbError>) -> Option<String> {
    decoded.ok().flatten().filter(|s| !s.trim().is_empty())
}

fn open_one(path: &Path) -> Option<Reader<Vec<u8>>> {
    if !path.exists() {
        return None;
    }
    match Reader::open_readfile(path) {
        Ok(reader) => Some(reader),
        Err(e) => {
            // A table that will not open is thrown away rather than kept: it will not open
            // next time either, and the next fetch replaces it.
            tracing::warn!(path = %path.display(), error = %e, "the table of places would not open");
            None
        }
    }
}

/// Where the tables live — beside the local database, in the person's own profile.
pub fn dir() -> Option<PathBuf> {
    directories::ProjectDirs::from("ru", "VRCast", "VRCast Studio")
        .map(|d| d.data_dir().to_path_buf())
}

/// Which month the tables on disk are from.
pub fn month_on_disk(dir: &Path) -> Option<String> {
    std::fs::read_to_string(dir.join(STAMP))
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}

/// The month to ask for, as DB-IP names it.
pub fn month_name(year: i32, month: u8) -> String {
    format!("{year:04}-{month:02}")
}

/// The month before a given one.
///
/// Needed because a month's file appears a little way into it: on the first of the month
/// the current one may not be published yet, and giving up then would leave a person
/// without any table for days.
pub fn previous_month(year: i32, month: u8) -> (i32, u8) {
    if month <= 1 {
        (year - 1, 12)
    } else {
        (year, month - 1)
    }
}

fn url(kind: &str, month: &str) -> String {
    format!("{BASE}/dbip-{kind}-lite-{month}.mmdb.gz")
}

/// Whether the tables need fetching.
pub fn needs_fetching(dir: &Path, now_month: &str) -> bool {
    if !dir.join(CITY).exists() || !dir.join(ASN).exists() {
        return true;
    }
    month_on_disk(dir).as_deref() != Some(now_month)
}

/// Fetch both tables for the newest month there is, and put them in place.
///
/// Comes back with the month that was taken. Quiet about failure by design: no network, or
/// a table that will not download, leaves the application working and every viewer "not
/// determined". Refusing to start over it would be absurd.
pub async fn fetch(dir: &Path, year: i32, month: u8) -> Result<String, String> {
    let months = [month_name(year, month), {
        let (y, m) = previous_month(year, month);
        month_name(y, m)
    }];

    let mut last = String::new();
    for wanted in &months {
        match fetch_month(dir, wanted).await {
            Ok(()) => {
                let _ = std::fs::write(dir.join(STAMP), wanted);
                return Ok(wanted.clone());
            }
            Err(e) => {
                // Falling back to the previous month is the ordinary path early in a month,
                // not a fault worth shouting about.
                tracing::debug!(month = %wanted, error = %e, "that month's tables are not there");
                last = e;
            }
        }
    }
    Err(last)
}

async fn fetch_month(dir: &Path, month: &str) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("could not make {}: {e}", dir.display()))?;
    for (kind, name) in [("city", CITY), ("asn", ASN)] {
        let bytes = download(&url(kind, month)).await?;
        let unpacked = gunzip(&bytes)?;
        // Written beside and then moved into place: a fetch cut off halfway would otherwise
        // leave half a table where a whole one is expected, and it would open and answer
        // nonsense rather than fail.
        let temp = dir.join(format!("{name}.part"));
        std::fs::write(&temp, &unpacked).map_err(|e| format!("could not write {name}: {e}"))?;
        std::fs::rename(&temp, dir.join(name))
            .map_err(|e| format!("could not put {name} in place: {e}"))?;
    }
    Ok(())
}

async fn download(url: &str) -> Result<Vec<u8>, String> {
    let response = reqwest::get(url)
        .await
        .map_err(|e| crate::store::redact::safe_display(&e))?;
    if !response.status().is_success() {
        return Err(format!("{url}: {}", response.status()));
    }
    response
        .bytes()
        .await
        .map(|b| b.to_vec())
        .map_err(|e| crate::store::redact::safe_display(&e))
}

fn gunzip(bytes: &[u8]) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(bytes)
        .read_to_end(&mut out)
        .map_err(|e| format!("the table would not unpack: {e}"))?;
    Ok(out)
}
