//! Contract tests for the command layer (T015).
//!
//! A test target of its own: the contract is a boundary, and it has to be checked apart
//! from the layers' internal tests.

#[path = "contract/support.rs"]
mod support;

#[path = "contract/basics.rs"]
mod basics;

#[path = "contract/contract_sync.rs"]
mod contract_sync;

#[path = "contract/library.rs"]
mod library;

#[path = "contract/responsiveness.rs"]
mod responsiveness;

#[path = "contract/secrets_never_returned.rs"]
mod secrets_never_returned;

#[path = "contract/servers.rs"]
mod servers;

#[path = "contract/upload.rs"]
mod upload;

#[path = "contract/convert.rs"]
mod convert;
