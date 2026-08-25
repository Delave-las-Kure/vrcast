//! T030 — a medium and a served file (`data-model.md` sections 3–4), the `slug` rules
//! included.
//!
//! Why a file has no `origin_url` and `cdn_url` fields although the data model has them: a
//! link is **worked out** from the server profile (see `links`) rather than stored beside
//! the file. A stored link goes quietly stale the day a person changes their domain or puts
//! a CDN in front — and the application starts handing out addresses that do not work,
//! which nobody learns until a viewer opens one.

use super::wording::{Detail, DetailCode};
use serde::{Deserialize, Serialize};

/// The length limit for a `slug`. A file name is put together as `<slug>_<bitrate>.mp4`,
/// and the file-name limit in file systems is 255 bytes; a hundred leaves room for the
/// suffixes and for the fact that non-Latin names take more than one byte per character.
pub const MAX_SLUG_LEN: usize = 100;

/// A medium — what a person considers one work.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Media {
    pub id: String,
    pub title: String,
    pub slug: String,
    /// The names of the served files, relative to the video directory.
    ///
    /// The default is there deliberately: the catalogue lies on the server and a person may
    /// have edited it, and a medium with not one file is a legitimate state rather than a
    /// reason to fail to read the catalogue whole.
    #[serde(default)]
    pub files: Vec<String>,
    /// The quality-ladder descriptions, relative to the video directory.
    #[serde(default)]
    pub ladders: Vec<String>,
    #[serde(default)]
    pub created_at: String,
    /// The fields this application does not know.
    ///
    /// They are kept when the catalogue is rewritten, deliberately — for the same reason as
    /// at the level of the whole catalogue (see `manifest::Manifest::extra`): a newer copy
    /// of the application may have created the medium, and throwing away what it recorded
    /// means losing it.
    #[serde(
        flatten,
        default,
        skip_serializing_if = "std::collections::HashMap::is_empty"
    )]
    pub extra: std::collections::HashMap<String, serde_json::Value>,
}

impl Media {
    /// A new medium with no files.
    pub fn new(
        id: impl Into<String>,
        title: impl Into<String>,
        slug: impl Into<String>,
        created_at: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            slug: slug.into(),
            files: Vec::new(),
            ladders: Vec::new(),
            created_at: created_at.into(),
            extra: std::collections::HashMap::new(),
        }
    }

    /// Every path the medium accounts for: both the files and the ladder descriptions.
    pub fn all_paths(&self) -> impl Iterator<Item = &String> {
        self.files.iter().chain(self.ladders.iter())
    }
}

/// A served file: the facts known about it.
///
/// Everything but `path`, `size_bytes` and `exists_on_server` may be unknown — the
/// parameters are got by parsing the MP4 header, and a file prepared by something other
/// than our own process may not have its header at the beginning (see `moov`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaFile {
    /// The path, relative to the video directory.
    pub path: String,
    pub size_bytes: u64,
    pub duration_s: Option<f64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// The average bitrate.
    pub bitrate_bps: Option<u64>,
    pub video_codec: Option<String>,
    pub audio_codec: Option<String>,
    /// `moov` was found at the beginning of the file. False means the file does not match
    /// the target format, and a viewer will wait for its tail to download before playback
    /// starts. `None` means it has not been checked yet.
    pub faststart_ok: Option<bool>,
    /// False means the file was deleted or renamed outside the application (FR-018).
    pub exists_on_server: bool,
}

impl MediaFile {
    /// A file about which only its existence and its size are known.
    pub fn known(path: impl Into<String>, size_bytes: u64) -> Self {
        Self {
            path: path.into(),
            size_bytes,
            duration_s: None,
            width: None,
            height: None,
            bitrate_bps: None,
            video_codec: None,
            audio_codec: None,
            faststart_ok: None,
            exists_on_server: true,
        }
    }

    /// Whether a link to this file is fit to hand to a viewer (FR-018).
    pub fn link_is_usable(&self) -> bool {
        self.exists_on_server
    }
}

/// What is wrong with a `slug`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlugError {
    Empty,
    TooLong { len: usize },
    BadChars { first_bad: char },
    Reserved,
}

impl SlugError {
    /// What to say about it. The wording belongs to the interface (FR-105, FR-106).
    pub fn detail(&self) -> Detail {
        match self {
            Self::Empty => Detail::new(DetailCode::SlugEmpty),
            Self::TooLong { len } => Detail::new(DetailCode::SlugTooLong)
                .with("len", *len)
                .with("max", MAX_SLUG_LEN),
            // The character goes out as text: naming it is the whole point, and a
            // person cannot find it in their input from a code alone.
            Self::BadChars { first_bad } => {
                Detail::new(DetailCode::SlugBadChar).with("char", first_bad.to_string())
            }
            Self::Reserved => Detail::new(DetailCode::SlugReserved),
        }
    }
}

/// Developer-facing, for logs and for `?`. What a person is shown comes from
/// [`SlugError::detail`] instead.
impl std::fmt::Display for SlugError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => f.write_str("slug is empty"),
            Self::TooLong { len } => write!(f, "slug is {len} bytes, limit is {MAX_SLUG_LEN}"),
            Self::BadChars { first_bad } => write!(f, "slug has a bad character: {first_bad:?}"),
            Self::Reserved => f.write_str("slug is reserved"),
        }
    }
}

/// Names that must not be taken: to the file system or to the serving they mean something
/// other than what they look like.
const RESERVED_SLUGS: &[&str] = &["_slow", ".", ".."];

/// Check a `slug`: Latin letters, digits, hyphen, underscore (`data-model.md` section 3).
pub fn validate_slug(slug: &str) -> Result<(), SlugError> {
    if slug.is_empty() {
        return Err(SlugError::Empty);
    }
    if let Some(bad) = slug
        .chars()
        .find(|c| !(c.is_ascii_alphanumeric() || *c == '-' || *c == '_'))
    {
        return Err(SlugError::BadChars { first_bad: bad });
    }
    // The length is counted after the characters are checked: a message about a
    // disallowed character is more useful than one about the length when both are wrong.
    if slug.len() > MAX_SLUG_LEN {
        return Err(SlugError::TooLong { len: slug.len() });
    }
    if RESERVED_SLUGS.contains(&slug) {
        return Err(SlugError::Reserved);
    }
    Ok(())
}

/// Make a `slug` out of a title.
///
/// A person's titles are in their own language while a `slug` goes into a file name and
/// into a link, so Cyrillic is transliterated into Latin. It returns `None` when there is
/// nothing to transliterate (a title made entirely of characters with no Latin counterpart)
/// — the short name must then be set by a person rather than by the application out of
/// rubbish.
pub fn slugify(title: &str) -> Option<String> {
    let mut out = String::with_capacity(title.len());
    let mut pending_separator = false;

    for ch in title.chars().flat_map(|c| c.to_lowercase()) {
        if let Some(latin) = transliterate(ch) {
            if !latin.is_empty() {
                if pending_separator && !out.is_empty() {
                    out.push('-');
                }
                pending_separator = false;
                out.push_str(latin);
            }
        } else if ch.is_ascii_alphanumeric() {
            if pending_separator && !out.is_empty() {
                out.push('-');
            }
            pending_separator = false;
            out.push(ch);
        } else {
            // Every other character is a separator. Separators do not pile up: spaces,
            // dots and dashes in a row give one hyphen rather than a chain.
            pending_separator = true;
        }
    }

    let trimmed = out.trim_matches('-').to_owned();
    if trimmed.is_empty() {
        return None;
    }

    let capped = cap_len(&trimmed, MAX_SLUG_LEN);
    if capped.is_empty() {
        None
    } else {
        Some(capped)
    }
}

/// Cut down to the limit without breaking a word in the middle where that can be avoided.
fn cap_len(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_owned();
    }
    let cut = &s[..max];
    match cut.rfind('-') {
        // Cut at a word boundary, but only when at least half the limit is left that way:
        // otherwise a long title is reduced to a stub.
        Some(i) if i >= max / 2 => cut[..i].to_owned(),
        _ => cut.trim_end_matches('-').to_owned(),
    }
}

/// The Latin counterpart of a Cyrillic letter.
///
/// `Some("")` is a letter written as nothing at all (the hard and soft signs). `None` means
/// it is not Cyrillic, and the caller decides.
fn transliterate(c: char) -> Option<&'static str> {
    Some(match c {
        'а' => "a",
        'б' => "b",
        'в' => "v",
        'г' => "g",
        'д' => "d",
        'е' => "e",
        'ё' => "e",
        'ж' => "zh",
        'з' => "z",
        'и' => "i",
        'й' => "y",
        'к' => "k",
        'л' => "l",
        'м' => "m",
        'н' => "n",
        'о' => "o",
        'п' => "p",
        'р' => "r",
        'с' => "s",
        'т' => "t",
        'у' => "u",
        'ф' => "f",
        'х' => "h",
        'ц' => "ts",
        'ч' => "ch",
        'ш' => "sh",
        'щ' => "sch",
        'ъ' => "",
        'ы' => "y",
        'ь' => "",
        'э' => "e",
        'ю' => "yu",
        'я' => "ya",
        // Ukrainian and Belarusian letters: a person may well use them in a title.
        'і' => "i",
        'ї' => "yi",
        'є' => "ye",
        'ґ' => "g",
        'ў' => "u",
        _ => return None,
    })
}
