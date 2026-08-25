//! Setting up log output.
//!
//! The one point where the log meets the output stream — and therefore the one place
//! to put secret redaction so that it covers everything (constitution, principle IV).

use crate::store::redact::RedactingMakeWriter;
use tracing_subscriber::EnvFilter;

/// Turn logging on. Called once, when the application starts.
///
/// Calling it again breaks nothing: installing the global subscriber quietly fails and
/// the arrangement already in force stays.
pub fn init() {
    let filter = EnvFilter::try_from_env("VRCAST_LOG").unwrap_or_else(|_| EnvFilter::new("info"));

    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        // All output passes through secret redaction before it is written.
        .with_writer(RedactingMakeWriter::new(std::io::stderr))
        .with_target(false)
        .try_init();
}
