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

pub mod convert_plan;
pub mod grouping;
pub mod links;
pub mod manifest;
pub mod media;
pub mod moov;
pub mod progress_estimate;
pub mod rate_limit;
pub mod remote_name;
pub mod server_profile;
pub mod source;
pub mod transfer;
pub mod wording;
