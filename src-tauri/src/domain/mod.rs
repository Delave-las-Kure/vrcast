//! The subject matter: rules that depend on neither the server nor the file system.
//!
//! There is no input, output or network here — only what can be tested without a
//! server (constitution, "limits on how work is done": logic that can only be checked
//! through a server counts as unchecked).
//!
//! One line runs through the split: **what is known** lives here, **how to find it
//! out** lives in the `ssh` and `server` layers. That is why, for instance, a served
//! file has no fields holding ready-made links: a link is worked out from the profile
//! (`links`) rather than stored beside the file, where it would inevitably go stale
//! the day the domain changes.

pub mod access_log;
pub mod chunks;
pub mod connections;
pub mod convert_plan;
pub mod deploy_steps;
pub mod dns_verdict;
pub mod geo;
pub mod grouping;
pub mod health;
pub mod hls_master;
pub mod hls_package;
pub mod ladder;
pub mod ladder_build;
pub mod limits_conf;
pub mod links;
pub mod log_digest;
pub mod manifest;
pub mod measure_grid;
pub mod measured_ladder;
pub mod media;
pub mod moov;
pub mod progress_estimate;
pub mod pseudonym;
pub mod rate_limit;
pub mod remote_name;
pub mod server_profile;
pub mod server_state;
pub mod slow_master;
pub mod source;
pub mod stalls;
pub mod swap;
pub mod transfer;
pub mod viewers;
pub mod wording;
