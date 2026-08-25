//! The application's stores.
//!
//! The split here is a matter of principle and follows from the constitution
//! (principle IV):
//!
//! | What | Where | Why |
//! |---|---|---|
//! | passwords, passphrases, keys | [`secrets`] — the OS store | must not sit in the application's files |
//! | profiles, tasks, cache, fingerprints | [`db`] — SQLite | survive a restart |
//! | cutting secrets out of output | [`redact`] | the guard stands at the exit, not at each call site |

pub mod db;
pub mod library_cache;
pub mod profiles;
pub mod redact;
pub mod secrets;
