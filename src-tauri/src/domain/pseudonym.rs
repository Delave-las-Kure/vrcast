//! T222 — a viewer's address in the log and in anything that goes outward (FR-057).
//!
//! **The decision, made by the owner on 2026-08-26: a stable pseudonym everywhere.**
//!
//! The two obvious answers were both wrong for this. Cutting addresses out entirely makes
//! "somebody says it stutters for them" impossible to look into — and looking into it is
//! half of what the viewers screen is for. Leaving them in makes an error report carry the
//! addresses of people who never agreed to anything, which is exactly what FR-057 forbids.
//!
//! So the log carries a short token instead: the same address always gives the same token,
//! so a person can see that this is the same viewer twice in an evening, or the same one
//! whose complaint they are reading about — while the address itself is not in the file.
//!
//! **What this is and is not.** The token is a keyed hash with a key made once on this
//! machine. Somebody who has the log and nothing else cannot get an address out of it, and
//! tokens from two different machines cannot be compared at all. Somebody who has the key
//! *and* a particular address to try can confirm a guess — that is the price of the token
//! staying the same across restarts, and the alternative (a key made afresh each run) turns
//! one viewer into a different stranger every time the application is opened.
//!
//! The interface is a different matter and shows real addresses: it is the owner's own
//! screen, and FR-057 is about third parties. This is about what is written down.

use sha2::{Digest, Sha256};

/// How much of the hash is shown.
///
/// Six characters — sixteen million possibilities. Enough that two viewers in one evening
/// will not collide, short enough to read at a glance and to say aloud.
const SHOWN: usize = 6;

/// What a viewer is called in writing.
pub fn pseudonym(ip: &str, key: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(key.as_bytes());
    hasher.update(b"\0");
    // Case and surrounding space must not make two names for one viewer: an address written
    // `2A00:…` in one place and `2a00:…` in another is the same address.
    hasher.update(ip.trim().to_ascii_lowercase().as_bytes());
    let digest = hasher.finalize();

    let mut out = String::from("viewer#");
    for byte in digest.iter().take(SHOWN.div_ceil(2)) {
        out.push_str(&format!("{byte:02x}"));
    }
    out.truncate("viewer#".len() + SHOWN);
    out
}

/// Replace every address in a piece of text with its pseudonym.
///
/// For the raw output of the server's own tools — the connection table, the access log —
/// which is full of addresses and reaches the log whole when something goes wrong. Naming
/// each address at the place it is used would miss those: they arrive as one lump of text
/// nobody looked inside.
///
/// **Deliberately not applied to all log output.** The person's own server has an address
/// too, and turning that into `viewer#…` in their own log would take away the one line that
/// says which machine they are talking to. This is for text that came from watching
/// viewers, and it is applied where that text is read.
pub fn scrub_addresses(text: &str, key: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;

    while !rest.is_empty() {
        match next_address(rest) {
            Some((at, len)) => {
                out.push_str(&rest[..at]);
                out.push_str(&pseudonym(&rest[at..at + len], key));
                rest = &rest[at + len..];
            }
            None => {
                out.push_str(rest);
                break;
            }
        }
    }
    out
}

/// Where the next address starts in this text, and how long it is.
fn next_address(text: &str) -> Option<(usize, usize)> {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // An address begins at a boundary: mid-word digits are part of something else.
        let at_boundary = i == 0 || !is_addressy(bytes[i - 1]);
        if at_boundary {
            if let Some(len) = address_len(&text[i..]) {
                return Some((i, len));
            }
        }
        i += 1;
    }
    None
}

/// A character that can be inside an address, used to find its edges.
fn is_addressy(b: u8) -> bool {
    b.is_ascii_hexdigit() || b == b'.' || b == b':'
}

/// How long the address at the start of this text is, if there is one.
fn address_len(text: &str) -> Option<usize> {
    let end = text
        .find(|c: char| !is_addressy(c as u8) || !c.is_ascii())
        .unwrap_or(text.len());
    let mut candidate = &text[..end];

    // A trailing dot or colon belongs to the sentence, not to the address.
    while candidate.ends_with('.') || candidate.ends_with(':') {
        candidate = &candidate[..candidate.len() - 1];
    }
    if candidate.is_empty() {
        return None;
    }

    // The address itself, and the address with a port on it. `ss` writes both, and an
    // address with a port must not be left half-scrubbed.
    if candidate.parse::<std::net::IpAddr>().is_ok() {
        return Some(candidate.len());
    }
    if let Some((host, port)) = candidate.rsplit_once(':') {
        if port.chars().all(|c| c.is_ascii_digit())
            && !port.is_empty()
            && host.parse::<std::net::Ipv4Addr>().is_ok()
        {
            return Some(host.len());
        }
    }
    None
}
