//! The core's unit tests.
//!
//! The files live in `tests/unit/` rather than beside this one: the path is given
//! explicitly, because a test target's root looks for modules in its own directory rather
//! than in a folder of the same name.

/// Shared fixtures. The process-state checks are needed by the tests for starting
/// programs and by those for the sweep alike: two copies of one check have already once
/// given different answers to the same question.
#[path = "support/proc_check.rs"]
mod proc_check;

/// The test key is made on the spot and is needed both here and by the integration tests.
#[path = "support/test_key.rs"]
mod test_key;

#[path = "unit/db.rs"]
mod db;

#[path = "unit/domain_us1.rs"]
mod domain_us1;

#[path = "unit/env_import.rs"]
mod env_import;

#[path = "unit/moov.rs"]
mod moov;

#[path = "unit/engine.rs"]
mod engine;

#[path = "unit/geo.rs"]
mod geo;

#[path = "unit/health.rs"]
mod health;

#[path = "unit/log_digest.rs"]
mod log_digest;

#[path = "unit/stalls.rs"]
mod stalls;

#[path = "unit/ladder.rs"]
mod ladder;

#[path = "unit/limits.rs"]
mod limits;

#[path = "unit/ladder_build.rs"]
mod ladder_build;

#[path = "unit/hls_package.rs"]
mod hls_package;

#[path = "unit/measured_ladder.rs"]
mod measured_ladder;

#[path = "unit/measurements.rs"]
mod measurements;

#[path = "unit/process.rs"]
mod process;

#[path = "unit/registry.rs"]
mod registry;

#[path = "unit/reconcile.rs"]
mod reconcile;

#[path = "unit/viewers.rs"]
mod viewers;

#[path = "unit/redact.rs"]
mod redact;

#[path = "unit/pseudonym.rs"]
mod pseudonym;

/// T353 — a database from an earlier build opens, migrates and keeps its rows. Here rather
/// than among the integration tests: it needs no container, and that set is off by default.
/// T354 — a forged update is refused: the shipped public key, the real plugin, and a
/// payload signed by a key that is not ours.
#[path = "unit/forged_update.rs"]
mod forged_update;

/// T351 — the update settings parse the way the plugin will parse them at startup: a
/// section it cannot read is not an application without updates, it is one that will not
/// open.
#[path = "unit/updater_config.rs"]
mod updater_config;

#[path = "unit/upgrade_keeps_state.rs"]
mod upgrade_keeps_state;

#[path = "unit/transfer.rs"]
mod transfer;

#[path = "unit/ssh.rs"]
mod ssh;

#[path = "unit/notify.rs"]
mod notify;

#[path = "unit/convert_plan.rs"]
mod convert_plan;

#[path = "unit/ffmpeg.rs"]
mod ffmpeg;

#[path = "unit/probe.rs"]
mod probe;

#[path = "unit/encoders.rs"]
mod encoders;

#[path = "unit/convert.rs"]
mod convert;

#[path = "unit/validate.rs"]
mod validate;

#[path = "unit/vmaf.rs"]
mod vmaf;

#[path = "unit/server_state.rs"]
mod server_state;

#[path = "unit/deploy_steps.rs"]
mod deploy_steps;

#[path = "unit/dns_verdict.rs"]
mod dns_verdict;

#[path = "unit/swap.rs"]
mod swap;

#[path = "unit/detect.rs"]
mod detect;

#[path = "unit/deploy_engine.rs"]
mod deploy_engine;

#[path = "unit/gate.rs"]
mod gate;

#[path = "unit/keygen.rs"]
mod keygen;

#[path = "unit/spawn_hygiene.rs"]
mod spawn_hygiene;

#[path = "unit/tray.rs"]
mod tray;

#[path = "unit/ladder_size.rs"]
mod ladder_size;

#[path = "unit/notices.rs"]
mod notices;

#[path = "unit/code_language.rs"]
mod code_language;
