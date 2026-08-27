//! Reaching the network from this machine, for things that are not the server.
//!
//! The one thing here so far is name resolution (FR-137…FR-140), and it is here rather than
//! in `server` because it asks the internet about a domain rather than asking a server about
//! itself — it runs before there is a server to ask.

pub mod dns;
