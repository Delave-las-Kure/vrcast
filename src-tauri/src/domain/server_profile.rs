//! T029 — the server profile and its checks (`data-model.md` section 1).
//!
//! A profile **contains no secret** — only a reference to an entry in the OS store
//! (constitution, principle IV). That is not a convention but a property of the type:
//! there is simply no field for a password here, and nowhere to put one.

use super::wording::{Detail, DetailCode};
use serde::{Deserialize, Serialize};

/// The default serving directory.
///
/// This is the one place in the whole application where a serving path is spelt out.
/// It is put into a new profile, is immediately editable, and from then on the
/// application takes the path **only** from the profile (FR-004). The mark at the end
/// of the line is what `scripts/check-no-hardcoded-server.sh` tells this default by,
/// as against a binding to somebody's server left there by accident.
pub const DEFAULT_VIDEO_DIR: &str = "/var/lib/vrcast/videos"; // FR-004-ok: the default

/// The default SSH port.
pub const DEFAULT_SSH_PORT: u16 = 22;

/// A limit on a profile name's length, so the list stays readable.
const MAX_NAME_LEN: usize = 100;

/// How to sign in to the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthKind {
    Key,
    Password,
}

impl AuthKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Key => "key",
            Self::Password => "password",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "key" => Some(Self::Key),
            "password" => Some(Self::Password),
            _ => None,
        }
    }
}

/// What to do about IPv6 when deploying (FR-135). `None` in a profile means the
/// person has not chosen yet; a silent default here is not acceptable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Ipv6Mode {
    Keep,
    Disable,
}

impl Ipv6Mode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Keep => "keep",
            Self::Disable => "disable",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "keep" => Some(Self::Keep),
            "disable" => Some(Self::Disable),
            _ => None,
        }
    }
}

/// A server profile. Kept in the local database.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerProfile {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub auth_kind: AuthKind,
    /// A reference to an entry in the OS store — **not the secret itself**.
    pub secret_ref: String,
    /// The path to the key file. Only when `auth_kind = Key`.
    pub key_path: Option<String>,
    /// The serving domain. Required: without it there is no link to hand out and no
    /// way to check that serving works (FR-125).
    pub domain: String,
    pub video_dir: String,
    /// Empty means links only from the origin (FR-016).
    pub cdn_base: Option<String>,
    pub host_fingerprint: Option<String>,
    pub ipv6_mode: Option<Ipv6Mode>,
    pub is_active: bool,
}

/// What exactly is wrong with a profile.
///
/// The check returns **every** objection at once rather than the first: in the setup
/// wizard a person fills the form in whole, and showing the errors one at a time means
/// sending them round again for each typo.
#[derive(Debug, Clone, PartialEq)]
pub struct ProfileProblem {
    /// Which field to highlight in the form.
    pub field: &'static str,
    /// What to say about it. The wording is the interface's (FR-105, FR-106).
    pub detail: Detail,
}

impl ProfileProblem {
    fn new(field: &'static str, key: DetailCode) -> Self {
        Self {
            field,
            detail: Detail::new(key),
        }
    }

    /// An objection that names a number: a length limit, an allowed range.
    fn with(field: &'static str, detail: Detail) -> Self {
        Self { field, detail }
    }
}

impl std::fmt::Display for ProfileProblem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.field, self.detail.key)
    }
}

impl ServerProfile {
    /// A new profile with sensible defaults. It still has to pass the checks.
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            host: String::new(),
            port: DEFAULT_SSH_PORT,
            user: String::from("root"),
            auth_kind: AuthKind::Key,
            secret_ref: String::new(),
            key_path: None,
            domain: String::new(),
            video_dir: String::from(DEFAULT_VIDEO_DIR),
            cdn_base: None,
            host_fingerprint: None,
            ipv6_mode: None,
            is_active: false,
        }
    }

    /// Bring the fields to canonical form: trim the edges, strip the scheme and a
    /// trailing slash from the domain, and a trailing slash from paths.
    ///
    /// Normalising is deliberately separate from checking. People paste a domain out
    /// of a browser's address bar — complete with `https://` and a slash. Refusing for
    /// that would be pedantry: the intent is unambiguous. A path with `..`, on the
    /// other hand, cannot be normalised — the check has something to say about it.
    pub fn normalize(&mut self) {
        self.id = self.id.trim().to_owned();
        self.name = self.name.trim().to_owned();
        self.host = self.host.trim().to_owned();
        self.user = self.user.trim().to_owned();
        self.secret_ref = self.secret_ref.trim().to_owned();
        self.domain = normalize_domain(&self.domain);
        self.video_dir = normalize_dir(&self.video_dir);

        self.key_path = self
            .key_path
            .take()
            .map(|p| p.trim().to_owned())
            .filter(|p| !p.is_empty());
        self.cdn_base = self
            .cdn_base
            .take()
            .map(|b| b.trim().trim_end_matches('/').to_owned())
            .filter(|b| !b.is_empty());
        self.host_fingerprint = self
            .host_fingerprint
            .take()
            .map(|f| f.trim().to_owned())
            .filter(|f| !f.is_empty());

        // A key means something only with key sign-in. Keeping it with password
        // sign-in means storing a path that will one day be applied to the wrong
        // profile.
        if self.auth_kind == AuthKind::Password {
            self.key_path = None;
        }
    }

    /// Check the whole profile. [`Self::normalize`] is worth calling first.
    pub fn validate(&self) -> Result<(), Vec<ProfileProblem>> {
        let mut problems = Vec::new();

        if self.id.trim().is_empty() {
            problems.push(ProfileProblem::new("id", DetailCode::ProfileIdEmpty));
        }

        if self.name.trim().is_empty() {
            problems.push(ProfileProblem::new("name", DetailCode::ProfileNameEmpty));
        } else if self.name.chars().count() > MAX_NAME_LEN {
            problems.push(ProfileProblem::with(
                "name",
                Detail::new(DetailCode::ProfileNameTooLong).with("max", MAX_NAME_LEN),
            ));
        }

        if self.host.is_empty() {
            problems.push(ProfileProblem::new("host", DetailCode::ProfileHostEmpty));
        } else if self.host.contains(char::is_whitespace) || self.host.contains('/') {
            problems.push(ProfileProblem::new("host", DetailCode::ProfileHostNotBare));
        }

        if self.port == 0 {
            problems.push(ProfileProblem::new("port", DetailCode::ProfilePortRange));
        }

        if self.user.is_empty() {
            problems.push(ProfileProblem::new("user", DetailCode::ProfileUserEmpty));
        } else if self.user.contains(char::is_whitespace) {
            problems.push(ProfileProblem::new(
                "user",
                DetailCode::ProfileUserHasSpaces,
            ));
        }

        if self.secret_ref.is_empty() {
            problems.push(ProfileProblem::new(
                "secret_ref",
                DetailCode::ProfileSecretRefEmpty,
            ));
        }

        match self.auth_kind {
            AuthKind::Key => {
                if self.key_path.as_deref().unwrap_or("").is_empty() {
                    problems.push(ProfileProblem::new(
                        "key_path",
                        DetailCode::ProfileKeyPathRequired,
                    ));
                }
            }
            AuthKind::Password => {
                if self.key_path.is_some() {
                    problems.push(ProfileProblem::new(
                        "key_path",
                        DetailCode::ProfileKeyPathUnused,
                    ));
                }
            }
        }

        if let Err(key) = check_domain(&self.domain) {
            problems.push(ProfileProblem::new("domain", key));
        }

        if let Err(key) = check_dir(&self.video_dir) {
            problems.push(ProfileProblem::new("video_dir", key));
        }

        if let Some(base) = &self.cdn_base {
            if let Err(key) = check_cdn_base(base) {
                problems.push(ProfileProblem::new("cdn_base", key));
            }
        }

        if problems.is_empty() {
            Ok(())
        } else {
            Err(problems)
        }
    }
}

/// Bring a domain to canonical form: no scheme, no trailing slash, lower case.
///
/// The case is taken down FIRST. Otherwise `HTTPS://…` pasted from an address bar does
/// not match the scheme pattern and stays inside the domain — and the link then
/// assembles with a doubled scheme and quietly stops working.
pub fn normalize_domain(raw: &str) -> String {
    let lowered = raw.trim().to_lowercase();
    let mut d = lowered.as_str();
    for prefix in ["https://", "http://"] {
        if let Some(rest) = d.strip_prefix(prefix) {
            d = rest;
            break;
        }
    }
    d.trim_end_matches('/').to_owned()
}

/// Bring a directory path to canonical form: no trailing slash.
fn normalize_dir(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.len() > 1 {
        trimmed.trim_end_matches('/').to_owned()
    } else {
        trimmed.to_owned()
    }
}

fn check_domain(domain: &str) -> Result<(), DetailCode> {
    if domain.is_empty() {
        return Err(DetailCode::DomainEmpty);
    }
    if domain.contains(char::is_whitespace) {
        return Err(DetailCode::DomainHasSpaces);
    }
    if domain.contains('/') {
        return Err(DetailCode::DomainHasPath);
    }
    if domain.contains('@') || domain.contains(':') {
        return Err(DetailCode::DomainHasUserOrPort);
    }
    if domain.starts_with('.') || domain.ends_with('.') || domain.contains("..") {
        return Err(DetailCode::DomainBadDots);
    }
    if !domain.contains('.') {
        return Err(DetailCode::DomainNoDot);
    }
    if domain
        .chars()
        .any(|c| !(c.is_alphanumeric() || c == '-' || c == '.'))
    {
        return Err(DetailCode::DomainBadChars);
    }
    Ok(())
}

fn check_dir(dir: &str) -> Result<(), DetailCode> {
    if dir.is_empty() {
        return Err(DetailCode::VideoDirEmpty);
    }
    if !dir.starts_with('/') {
        return Err(DetailCode::VideoDirNotAbsolute);
    }
    // `..` segments are not a theoretical danger: a path from here goes into commands
    // on the server, and one such segment takes a write outside the serving directory.
    if dir.split('/').any(|part| part == "..") {
        return Err(DetailCode::VideoDirHasDotDot);
    }
    if dir.contains('\n') || dir.contains('\r') {
        return Err(DetailCode::VideoDirHasNewline);
    }
    Ok(())
}

fn check_cdn_base(base: &str) -> Result<(), DetailCode> {
    if !(base.starts_with("https://") || base.starts_with("http://")) {
        return Err(DetailCode::CdnBaseNoScheme);
    }
    if base.contains(char::is_whitespace) {
        return Err(DetailCode::CdnBaseHasSpaces);
    }
    let rest = base
        .strip_prefix("https://")
        .or_else(|| base.strip_prefix("http://"))
        .unwrap_or("");
    if rest.is_empty() {
        return Err(DetailCode::CdnBaseIncomplete);
    }
    Ok(())
}
