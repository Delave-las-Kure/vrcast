//! T043 — a one-off carry-over of settings from `server.env`.
//!
//! `server.env` is what the author used before this application existed, and it keeps
//! working: the skills read it as they always did (constitution, principle VII). The
//! application offers to **carry** its values into the first profile, so that nobody
//! has to type in again what is already written down.
//!
//! Three rules, all three of them substantive:
//!
//! 1. **Read only.** The file is not modified or rewritten under any circumstances: it
//!    belongs to the old way of working, not to the application.
//! 2. **Once.** After the profile is created the application never returns to the
//!    file. Otherwise there would be two sources of truth, and an edit in the
//!    application would quietly diverge from the file.
//! 3. **The password is not carried over.** In the file it is usually empty, and if it
//!    is not, it is the fallback way in through the hosting provider's console rather
//!    than something the application should use. The key's passphrase a person enters
//!    themselves: it is not in the file.

use crate::commands::servers::ServerInput;
use crate::domain::server_profile::{AuthKind, DEFAULT_VIDEO_DIR};
use std::path::{Path, PathBuf};

/// What could be read out of `server.env`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Imported {
    /// Profile fields, ready to use. A person sees them in the wizard and can correct them.
    pub input: ServerInput,
    /// Where it came from — shown to the person so they understand what is happening.
    pub source: PathBuf,
    /// Whether a passphrase is needed: there is a key, and the file has no passphrase
    /// and cannot have one.
    pub needs_passphrase: bool,
}

/// Where to look for `server.env`, relative to the application directory.
///
/// The application lives in `vrcast-studio/`, and the file sits beside it, at the root
/// of the old way of working.
pub fn default_location() -> Option<PathBuf> {
    let exe = std::env::current_dir().ok()?;
    for dir in exe.ancestors().take(4) {
        let candidate = dir.join("server.env");
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// Read `server.env` and assemble the profile fields.
///
/// Returns `None` when there is no file, or when the essentials — the address and the
/// domain — are not in it. A missing file is not an error: most people using this
/// application will never have one.
pub fn read_from(path: &Path) -> Option<Imported> {
    let text = std::fs::read_to_string(path).ok()?;
    let values = parse(&text);

    let host = values.get("SERVER_IP").cloned().unwrap_or_default();
    let domain = values.get("SERVER_DOMAIN").cloned().unwrap_or_default();
    if host.is_empty() || domain.is_empty() {
        return None;
    }

    let key_path = values
        .get("SSH_KEY")
        .map(|k| expand_home(k))
        .filter(|k| !k.is_empty());

    Some(Imported {
        input: ServerInput {
            name: domain.clone(),
            host,
            port: 22,
            user: values
                .get("SSH_USER")
                .cloned()
                .filter(|u| !u.is_empty())
                .unwrap_or_else(|| String::from("root")),
            // Key sign-in, even when the file also names a password: a password
            // there is the fallback way in through the provider's console, not a
            // working method.
            auth_kind: if key_path.is_some() {
                AuthKind::Key
            } else {
                AuthKind::Password
            },
            key_path: key_path.clone(),
            domain,
            video_dir: values
                .get("VIDEO_DIR")
                .cloned()
                .filter(|d| !d.is_empty())
                .or_else(|| Some(String::from(DEFAULT_VIDEO_DIR))),
            cdn_base: values.get("CDN_BASE").cloned().filter(|c| !c.is_empty()),
            ipv6_mode: None,
        },
        source: path.to_path_buf(),
        needs_passphrase: key_path.is_some(),
    })
}

/// Parse a file of the form `KEY="value"`.
///
/// This is not a full shell and must not be one: command substitutions and branches do
/// not occur in such a file, and executing its contents in order to read it would mean
/// running someone else's code for the sake of four lines of settings.
fn parse(text: &str) -> std::collections::HashMap<String, String> {
    let mut out = std::collections::HashMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, rest)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim().trim_start_matches("export ").trim();
        if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            continue;
        }

        // A trailing comment is cut only outside quotes: inside a value a hash is
        // legitimate, and cutting at it blindly ruins paths and passwords.
        let value = strip_value(rest.trim());
        out.insert(key.to_owned(), value);
    }
    out
}

fn strip_value(raw: &str) -> String {
    let raw = raw.trim();
    let (quote, body) = match raw.chars().next() {
        Some(q @ ('"' | '\'')) => (Some(q), &raw[q.len_utf8()..]),
        _ => (None, raw),
    };

    match quote {
        Some(q) => match body.find(q) {
            Some(end) => body[..end].to_owned(),
            None => body.to_owned(),
        },
        None => body
            .split_once('#')
            .map_or(body, |(v, _)| v)
            .trim()
            .to_owned(),
    }
}

/// Expand `$HOME` and `~` — that is exactly how the key path is written in the file.
fn expand_home(value: &str) -> String {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_default();
    if home.is_empty() {
        return value.to_owned();
    }
    value
        .replace("$HOME", &home)
        .replace("${HOME}", &home)
        .replacen("~/", &format!("{home}/"), 1)
}
