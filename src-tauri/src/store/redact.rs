//! T010 — cutting secrets out of everything the application puts outside.
//!
//! Constitution, principle IV: "Secrets MUST NOT reach logs, error reports or diagnostic
//! dumps." Relying on every line of code being careful will not do here: one
//! `tracing::debug!("{:?}", profile)` somewhere deep is enough to put a password to
//! someone else's server into a log file. So the protection does not stand at the places
//! that print — it stands at the exit itself, which everything passes through.
//!
//! It works like this: every secret that enters the application is registered here (by
//! `store::secrets`, not by the calling code), and from then on every line going to the log
//! is put through the substitution before it is written.

use std::borrow::Cow;
use std::io::{self, Write};
use std::sync::{OnceLock, RwLock};

/// What a secret found in the text is replaced with. Deliberately conspicuous: if this
/// turned up in a log, the protection worked — the line does not merely "look odd".
pub const MASK: &str = "[secret hidden]";

/// Nothing shorter than this is registered. A three-character secret would turn up in a
/// log by chance hundreds of times and riddle it with masks, hiding everything useful along
/// with it.
const MIN_SECRET_LEN: usize = 8;

fn registry() -> &'static RwLock<Vec<String>> {
    static REGISTRY: OnceLock<RwLock<Vec<String>>> = OnceLock::new();
    REGISTRY.get_or_init(|| RwLock::new(Vec::new()))
}

/// Remember a value that must never appear in the output.
///
/// Called where a secret enters the application — not where it is printed. Values that are
/// too short are ignored on purpose (see `MIN_SECRET_LEN`).
pub fn register(secret: &str) {
    if secret.len() < MIN_SECRET_LEN {
        return;
    }
    let Ok(mut list) = registry().write() else {
        return; // a poisoned lock: quietly skip registering, but do not bring the app down
    };
    if list.iter().any(|s| s == secret) {
        return;
    }
    list.push(secret.to_owned());
    // The long ones first: if one secret contains another, the substitution starts with
    // the long one, or a tail of it would be left behind.
    list.sort_by_key(|b| std::cmp::Reverse(b.len()));
}

/// Forget ONE secret — the one that no longer exists.
///
/// This is how forgetting must go when a profile is deleted. The temptation to call
/// `forget_all` here is strong and dangerous: the other profiles' secrets are still alive,
/// and taking the masking off them would mean someone else's password reaching the log on
/// the very next error — until the application is restarted, and with no sign at all that
/// anything is wrong (debt T073, principle IV).
pub fn forget(secret: &str) {
    if secret.len() < MIN_SECRET_LEN {
        return;
    }
    if let Ok(mut list) = registry().write() {
        list.retain(|s| s != secret);
    }
}

/// Forget every registered secret.
///
/// **For tests only.** Calling this in a running application is forbidden: the other
/// profiles' secrets would stop being cut out of the output. For deleting a single profile
/// there is [`forget`].
pub fn forget_all() {
    if let Ok(mut list) = registry().write() {
        list.clear();
    }
}

/// Replace every registered secret and every private key block in the text.
///
/// It returns a `Cow` so that in the usual case — no secrets in the text — the string is
/// not copied: every line of the log goes through this function.
pub fn redact(text: &str) -> Cow<'_, str> {
    let mut out = redact_key_blocks(text);

    if let Ok(list) = registry().read() {
        for secret in list.iter() {
            if out.contains(secret.as_str()) {
                out = Cow::Owned(out.replace(secret.as_str(), MASK));
            }
        }
    }
    out
}

/// The markers of a private key block. Shared by the substitution and by the line-by-line
/// writer: it has to recognise a block that has begun so it never lets the body out in
/// pieces.
const KEY_BEGIN: &str = "-----BEGIN";
const KEY_END: &str = "-----END";

/// A private key is recognised by its shape rather than by its registration.
///
/// This is the safety net for when a key was read from a file and reached the output past
/// the secret store — that is, in exactly the case where registration did not happen.
fn redact_key_blocks(text: &str) -> Cow<'_, str> {
    const BEGIN: &str = KEY_BEGIN;
    const END_MARK: &str = KEY_END;

    if !text.contains(BEGIN) {
        return Cow::Borrowed(text);
    }

    let mut out = String::with_capacity(text.len());
    let mut rest = text;

    while let Some(start) = rest.find(BEGIN) {
        out.push_str(&rest[..start]);
        let tail = &rest[start..];

        // Looking for the end of the block: "-----END ... -----"
        match tail.find(END_MARK).and_then(|e| {
            tail[e + END_MARK.len()..]
                .find("-----")
                .map(|d| e + END_MARK.len() + d + 5)
        }) {
            Some(end) => {
                out.push_str(MASK);
                rest = &tail[end..];
            }
            None => {
                // The block begins and never ends — truncated output. Everything left is
                // hidden: better to lose the tail of the log than to show half a key.
                out.push_str(MASK);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    Cow::Owned(out)
}

/// A safe rendering of an error, for showing and for writing down.
///
/// Errors are the second most common path for a leak after the log: a secret reaches the
/// message through a `{}` from an underlying library that knows nothing of our rules.
pub fn safe_display<E: std::fmt::Display + ?Sized>(err: &E) -> String {
    redact(&err.to_string()).into_owned()
}

/// A wrapper over an output stream that cuts secrets out before writing.
///
/// It gathers the output line by line: a write can arrive in pieces, and a secret torn
/// between two calls would otherwise slip past the substitution.
pub struct RedactingWriter<W: Write> {
    inner: W,
    buf: Vec<u8>,
}

impl<W: Write> RedactingWriter<W> {
    pub fn new(inner: W) -> Self {
        Self {
            inner,
            buf: Vec::new(),
        }
    }

    fn flush_complete_lines(&mut self) -> io::Result<()> {
        loop {
            let Some(pos) = self.buf.iter().position(|b| *b == b'\n') else {
                return Ok(());
            };

            let first_line = String::from_utf8_lossy(&self.buf[..=pos]);
            if !first_line.contains(KEY_BEGIN) {
                let line: Vec<u8> = self.buf.drain(..=pos).collect();
                let text = String::from_utf8_lossy(&line);
                self.inner.write_all(redact(&text).as_bytes())?;
                continue;
            }

            // The line begins a private key block. Letting it out line by line will not
            // do: the body of a key is lines of plain base64, each indistinguishable on its
            // own from harmless output, and a block can only be recognised whole. Until the
            // end of the block arrives the buffer is held (flush masks the remainder as a
            // truncated block).
            if has_unclosed_key_block(&String::from_utf8_lossy(&self.buf)) {
                return Ok(());
            }

            // The end of the block is in the buffer: every gathered complete line goes out
            // as one piece through one substitution — it sees the whole block and masks
            // it.
            let last_nl = self.buf.iter().rposition(|b| *b == b'\n').unwrap_or(pos);
            let chunk: Vec<u8> = self.buf.drain(..=last_nl).collect();
            let text = String::from_utf8_lossy(&chunk);
            self.inner.write_all(redact(&text).as_bytes())?;
        }
    }
}

/// Whether the text holds a private key block that has begun but not yet closed.
fn has_unclosed_key_block(text: &str) -> bool {
    let Some(start) = text.rfind(KEY_BEGIN) else {
        return false;
    };
    let tail = &text[start..];
    match tail.find(KEY_END) {
        Some(e) => !tail[e + KEY_END.len()..].contains("-----"),
        None => true,
    }
}

impl<W: Write> Write for RedactingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.buf.extend_from_slice(buf);
        self.flush_complete_lines()?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.flush_complete_lines()?;
        if !self.buf.is_empty() {
            let tail = std::mem::take(&mut self.buf);
            let text = String::from_utf8_lossy(&tail);
            self.inner.write_all(redact(&text).as_bytes())?;
        }
        self.inner.flush()
    }
}

impl<W: Write> Drop for RedactingWriter<W> {
    fn drop(&mut self) {
        let _ = self.flush();
    }
}

/// A source of output streams for `tracing` that wraps each one in secret redaction.
pub struct RedactingMakeWriter<M> {
    inner: M,
}

impl<M> RedactingMakeWriter<M> {
    pub fn new(inner: M) -> Self {
        Self { inner }
    }
}

impl<'a, M> tracing_subscriber::fmt::MakeWriter<'a> for RedactingMakeWriter<M>
where
    M: tracing_subscriber::fmt::MakeWriter<'a>,
{
    type Writer = RedactingWriter<M::Writer>;

    fn make_writer(&'a self) -> Self::Writer {
        RedactingWriter::new(self.inner.make_writer())
    }
}
