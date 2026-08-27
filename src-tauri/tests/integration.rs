//! Integration tests against a throwaway server in a container (T026).
//!
//! Off by default: they need Docker and run noticeably longer than the unit tests.
//! To run them:
//!
//! ```text
//! cargo test --features integration --test integration -- --test-threads=1
//! ```
//!
//! Running on one thread is not optional: each test brings up a container of its own, and
//! starting a dozen containers at once on an ordinary machine only slows things down.
//!
//! The live server is NEVER used here — constitution, the "Way of working" section.

#![cfg(feature = "integration")]

/// Shared fixtures: the test key is made on the spot and is needed both here and by the
/// unit tests.
#[path = "support/test_key.rs"]
mod test_key;

/// The process-state checks — shared with the unit tests.
#[path = "support/proc_check.rs"]
mod proc_check;

#[path = "integration/audio_sync.rs"]
mod audio_sync;

#[path = "integration/channels.rs"]
mod channels;

#[path = "integration/convert_kill.rs"]
mod convert_kill;

/// A bare server and somebody else's, for the deployment checks (T247). Kept apart from
/// `fixture` because it brings its containers up differently — with systemd as PID 1 — and
/// because it is the one fixture that guards the address it hands out (T249).
#[path = "integration/deploy_fixture.rs"]
mod deploy_fixture;

#[path = "integration/deploy_clean.rs"]
mod deploy_clean;

#[path = "integration/deploy_stand.rs"]
mod deploy_stand;

#[path = "integration/detect_live.rs"]
mod detect_live;

#[path = "integration/dns_live.rs"]
mod dns_live;

#[path = "integration/fixture.rs"]
mod fixture;

/// The tables of places, against the real ones (T162). Ignored by default: it downloads
/// about seventy megabytes.
#[path = "integration/geo_real.rs"]
mod geo_real;

/// A ready quality set in the container (T151). Used by the checks of Phases 4 and 6.
#[path = "integration/hls_fixture.rs"]
#[allow(dead_code)]
mod hls_fixture;

#[path = "integration/library_completeness.rs"]
mod library_completeness;

#[path = "integration/library_ops.rs"]
mod library_ops;

#[path = "integration/live_readonly.rs"]
mod live_readonly;

#[path = "integration/manifest_conflict.rs"]
mod manifest_conflict;

#[path = "integration/process_tree.rs"]
mod process_tree;

#[path = "integration/upload_live.rs"]
mod upload_live;

#[path = "integration/scenario_upload.rs"]
mod scenario_upload;

/// A check of the fixture itself (T149, T151, T152): that it serves, writes its log and
/// gives every viewer an address of their own.
#[path = "integration/serving.rs"]
mod serving;

#[path = "integration/hls_live.rs"]
mod hls_live;

#[path = "integration/limits_live.rs"]
mod limits_live;

#[path = "integration/seams.rs"]
mod seams;

#[path = "integration/quality_live.rs"]
mod quality_live;

#[path = "integration/ssh_live.rs"]
mod ssh_live;

#[path = "integration/viewers_live.rs"]
mod viewers_live;

/// The viewer helpers (T152). Used by the checks of Phases 4 and 6; until those are
/// written it is only compiled, and that is deliberate — a helper nobody has compiled is
/// not a helper.
#[path = "integration/viewer.rs"]
#[allow(dead_code)]
mod viewer;
