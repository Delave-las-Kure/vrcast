//! What each version of the server side put on a server.
//!
//! Kept so that an upgrade can tell **our older file** from **a file somebody edited by
//! hand**. Both differ from what we are about to write, and the two call for opposite
//! actions: ours is replaced without a word, theirs must never be (the ownership rule of
//! `contracts/server-contract.md`) — a person who tuned their own web server and found the
//! application had quietly undone it would be right to stop trusting it.
//!
//! With one version in existence this holds exactly one entry, and the distinction it draws
//! is dormant. It is written now because the moment it stops being dormant is the moment
//! version 2 appears — and by then nobody remembers that the difference mattered.

/// The main configuration as each version wrote it, newest last.
///
/// The domain is still a placeholder here: what is compared is the file with this server's
/// domain put in, and every version's reference goes through the same substitution.
const CADDYFILE_BY_VERSION: [(u32, &str); 1] =
    [(1, include_str!("../../../resources/server/Caddyfile"))];

/// The main configuration of a given version, with a domain put in.
pub fn caddyfile(version: u32, domain: &str) -> Option<String> {
    CADDYFILE_BY_VERSION
        .iter()
        .find(|(v, _)| *v == version)
        .map(|(_, text)| text.replace("{$SERVER_DOMAIN}", domain))
}

/// Was this file written by some version of this application?
///
/// The question an upgrade has to answer before replacing anything. A file that matches no
/// version of ours belongs to whoever wrote it.
pub fn is_ours(text: &str, domain: &str) -> bool {
    CADDYFILE_BY_VERSION
        .iter()
        .any(|(_, reference)| reference.replace("{$SERVER_DOMAIN}", domain) == text)
}

/// Every version this application knows how to have written.
pub fn known_versions() -> Vec<u32> {
    CADDYFILE_BY_VERSION.iter().map(|(v, _)| *v).collect()
}
