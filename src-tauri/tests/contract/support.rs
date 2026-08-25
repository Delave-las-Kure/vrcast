//! Shared fixtures for the contract tests.
//!
//! The application state is assembled with an in-memory database and an in-memory secret
//! store: a test that leaves entries behind in a person's system password manager is a bad
//! test.

use std::sync::Arc;
use vrcast_studio_lib::commands::servers::ServerInput;
use vrcast_studio_lib::commands::AppState;
use vrcast_studio_lib::domain::server_profile::AuthKind;
use vrcast_studio_lib::store::db::Db;
use vrcast_studio_lib::store::secrets::InMemorySecretStore;

pub fn state() -> AppState {
    AppState::with_db(
        Arc::new(Db::open_in_memory().unwrap()),
        Arc::new(InMemorySecretStore::new()),
    )
    .expect("the application state would not assemble")
}

/// Profile fields that are certainly fit. Each test changes what it checks and takes the
/// rest from here.
pub fn valid_input(name: &str) -> ServerInput {
    ServerInput {
        name: name.to_owned(),
        // An address from the block set aside for examples in documentation: it leads to
        // nobody's real server.
        host: String::from("203.0.113.10"),
        port: 22,
        user: String::from("root"),
        auth_kind: AuthKind::Password,
        key_path: None,
        domain: String::from("stream.example.com"),
        video_dir: None,
        cdn_base: None,
        ipv6_mode: None,
    }
}
