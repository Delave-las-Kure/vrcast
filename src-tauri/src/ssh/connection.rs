//! T022 — the connection to a server: one per server, channels inside it.
//!
//! Why one (R-04): a server limits how many connections may be established at once
//! (`maxstartups 10:30:100`). A flood of unauthenticated connections fills those slots and
//! new long sessions are dropped — that is exactly how the author's ladder build once broke
//! halfway through the third variant. Watching a log, transferring a file and running short
//! commands go down separate channels inside one connection rather than down separate
//! connections.

use super::fingerprint::{client_config, ClientHandler, HostKeyPolicy, HostKeySlot};
use super::{auth, Credentials, Result, ServerAddress, SshError};
use russh::client;
use std::sync::{Arc, Mutex};
use tokio::sync::Semaphore;

/// How many channels are kept open at once inside one connection.
///
/// OpenSSH has a `MaxSessions` limit, **10** by default, and going over it gives not a
/// queue but a refusal: `Failed to open channel (ConnectFailed)`. Checked on a live server
/// on 2026-08-25 — twelve simultaneous channels would not open.
///
/// Eight is taken, leaving room to spare: a person's limit may be lower than the default,
/// and running into it must show up as waiting rather than as a refusal mid-work.
const MAX_CONCURRENT_CHANNELS: usize = 8;

/// An established connection to a server.
///
/// Cloning is cheap: the clones share one connection rather than making new ones.
#[derive(Clone)]
pub struct Connection {
    handle: Arc<client::Handle<ClientHandler>>,
    addr: ServerAddress,
    user: String,
    /// Caps how many channels are open at once — see `MAX_CONCURRENT_CHANNELS`.
    channels: Arc<Semaphore>,
}

impl Connection {
    /// Connect and log in to the server.
    ///
    /// `expected_fingerprint` is not optional: credentials are never sent to a server whose
    /// fingerprint has not been confirmed (see `fingerprint`). To learn the fingerprint
    /// beforehand, use [`super::fingerprint::probe`].
    pub async fn connect(
        addr: ServerAddress,
        user: impl Into<String>,
        credentials: Credentials,
        expected_fingerprint: &str,
    ) -> Result<Self> {
        let user = user.into();
        let slot = Arc::new(Mutex::new(HostKeySlot::default()));
        let handler = ClientHandler::new(
            HostKeyPolicy::Require(expected_fingerprint.to_owned()),
            slot.clone(),
        );

        let connect_result =
            client::connect(client_config(), (addr.host.as_str(), addr.port), handler).await;

        // Sorting out a failure starts with the server's key: a substitution matters more
        // than any other cause, and must be reported as itself rather than as a general
        // protocol error.
        let mut handle = match connect_result {
            Ok(h) => h,
            Err(e) => {
                let slot = slot.lock().ok();
                if let Some(slot) = slot {
                    if slot.was_certificate {
                        return Err(SshError::HostKeyIsCertificate);
                    }
                    if let Some((expected, actual)) = slot.mismatch.clone() {
                        return Err(SshError::HostKeyChanged {
                            addr,
                            expected,
                            actual,
                        });
                    }
                }
                return Err(SshError::Unreachable {
                    addr,
                    reason: crate::store::redact::safe_display(&e),
                });
            }
        };

        Self::authenticate(&mut handle, &user, credentials).await?;

        tracing::info!(server = %addr, user = %user, "connected to the server");

        Ok(Self {
            handle: Arc::new(handle),
            addr,
            user,
            channels: Arc::new(Semaphore::new(MAX_CONCURRENT_CHANNELS)),
        })
    }

    async fn authenticate(
        handle: &mut client::Handle<ClientHandler>,
        user: &str,
        credentials: Credentials,
    ) -> Result<()> {
        let result = match credentials {
            Credentials::Key { path, passphrase } => {
                let key = auth::load_key(&path, passphrase.as_deref())?;
                // The signature hash for RSA is chosen by what the server supports: many
                // servers no longer accept the old sha1.
                let hash_alg = handle
                    .best_supported_rsa_hash()
                    .await
                    .map_err(SshError::protocol)?
                    .flatten();
                handle
                    .authenticate_publickey(
                        user,
                        russh::keys::PrivateKeyWithHashAlg::new(Arc::new(key), hash_alg),
                    )
                    .await
                    .map_err(SshError::protocol)?
            }
            Credentials::Password(password) => {
                crate::store::redact::register(&password);
                handle
                    .authenticate_password(user, password)
                    .await
                    .map_err(SshError::protocol)?
            }
        };

        match result {
            russh::client::AuthResult::Success => Ok(()),
            russh::client::AuthResult::Failure {
                remaining_methods, ..
            } => {
                // The list of methods the server offered is not decoration for a report.
                // It is what shows the difference between "wrong password" and "password
                // login is forbidden for this user", which look the same from outside.
                let methods = remaining_methods
                    .iter()
                    .map(|m| format!("{m:?}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                Err(SshError::AuthFailed {
                    methods: if methods.is_empty() {
                        String::from("the server named none")
                    } else {
                        methods
                    },
                })
            }
        }
    }

    /// Open a session channel, surviving an instant refusal from the server.
    ///
    /// A session slot on the server is not freed at the moment `close` is exchanged: sshd
    /// first reaps the child process, and the signal that it finished arrives
    /// asynchronously. So even while our own limit (`MAX_CONCURRENT_CHANNELS`) is honoured,
    /// a fresh channel can run into a session that is spent but not yet reaped — the server
    /// answers with a refusal although the place is about to be free. Such a refusal is a
    /// reason to wait and retry rather than an error: seen live on 2026-08-25, when twelve
    /// queued through a semaphore of eight and still caught
    /// `ChannelOpenFailure(ConnectFailed)`.
    pub(crate) async fn open_session(&self) -> Result<russh::Channel<client::Msg>> {
        use russh::ChannelOpenFailure::{ConnectFailed, ResourceShortage};

        let mut delay = std::time::Duration::from_millis(50);
        let mut attempts_left = 8;
        loop {
            match self.handle.channel_open_session().await {
                Ok(channel) => return Ok(channel),
                Err(russh::Error::ChannelOpenFailure(
                    reason @ (ConnectFailed | ResourceShortage),
                )) if attempts_left > 0 => {
                    attempts_left -= 1;
                    tracing::debug!(
                        ?reason,
                        attempts_left,
                        "the server refused to open a channel; waiting and retrying"
                    );
                    tokio::time::sleep(delay).await;
                    delay = (delay * 2).min(std::time::Duration::from_secs(1));
                }
                Err(e) => return Err(SshError::protocol(e)),
            }
        }
    }

    /// Take a place for a channel, waiting in the queue when there is one.
    ///
    /// Waiting is more right here than refusing: going over the server's limit is not a
    /// person's mistake and not a reason to break off the work.
    pub(crate) async fn acquire_channel(&self) -> Result<tokio::sync::OwnedSemaphorePermit> {
        self.channels
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| SshError::Protocol(String::from("the connection is closing")))
    }

    pub fn address(&self) -> &ServerAddress {
        &self.addr
    }

    pub fn user(&self) -> &str {
        &self.user
    }

    /// Whether the connection is alive. Cheap: it does not touch the network.
    pub fn is_alive(&self) -> bool {
        !self.handle.is_closed()
    }

    /// Close the connection politely.
    pub async fn close(&self) {
        let _ = self
            .handle
            .disconnect(russh::Disconnect::ByApplication, "", "en")
            .await;
    }
}

impl std::fmt::Debug for Connection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Connection")
            .field("addr", &self.addr)
            .field("user", &self.user)
            .field("alive", &self.is_alive())
            .finish()
    }
}
