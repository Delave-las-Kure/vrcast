//! T022 — соединение с сервером: одно на сервер, каналы внутри него.
//!
//! Почему одно (R-04): сервер ограничивает число одновременно устанавливаемых соединений
//! (`maxstartups 10:30:100`). Флуд неаутентифицированных соединений забивает эти слоты, и
//! новые долгие сессии отбрасываются — именно так у автора однажды оборвалась сборка лесенки
//! на середине третьего варианта. Слежение за журналом, передача файла и короткие команды
//! идут отдельными каналами внутри одного соединения, а не отдельными соединениями.

use super::fingerprint::{client_config, ClientHandler, HostKeyPolicy, HostKeySlot};
use super::{auth, Credentials, Result, ServerAddress, SshError};
use russh::client;
use std::sync::{Arc, Mutex};
use tokio::sync::Semaphore;

/// Сколько каналов держим открытыми одновременно в одном соединении.
///
/// У OpenSSH есть предел `MaxSessions`, по умолчанию **10**, и превышение даёт не
/// очередь, а отказ: `Failed to open channel (ConnectFailed)`. Проверено на живом
/// сервере 2026-08-25 — двенадцать одновременных каналов не открылись.
///
/// Берём восемь, оставляя запас: у пользователя предел может быть ниже умолчания,
/// а вылезти за него мы обязаны не отказом посреди работы, а ожиданием.
const MAX_CONCURRENT_CHANNELS: usize = 8;

/// Установленное соединение с сервером.
///
/// Клонируется дёшево: клоны делят одно соединение, а не создают новые.
#[derive(Clone)]
pub struct Connection {
    handle: Arc<client::Handle<ClientHandler>>,
    addr: ServerAddress,
    user: String,
    /// Ограничитель числа одновременно открытых каналов — см. `MAX_CONCURRENT_CHANNELS`.
    channels: Arc<Semaphore>,
}

impl Connection {
    /// Подключиться и войти на сервер.
    ///
    /// `expected_fingerprint` обязателен: учётные данные не отправляются серверу, отпечаток
    /// которого не подтверждён (см. `fingerprint`). Узнать отпечаток заранее —
    /// [`super::fingerprint::probe`].
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

        // Разбор неудачи начинается с ключа сервера: подмена важнее любой другой причины,
        // и сообщать о ней надо ею, а не общей ошибкой протокола.
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

        tracing::info!(server = %addr, user = %user, "соединение с сервером установлено");

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
                // Хеш подписи для RSA выбирается по тому, что поддерживает сервер:
                // старый sha1 многие серверы уже не принимают.
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
                // Перечень предложенных сервером способов — не украшение отчёта.
                // Именно он показывает разницу между «неверный пароль» и «вход по паролю
                // для этого пользователя запрещён», а снаружи они выглядят одинаково.
                let methods = remaining_methods
                    .iter()
                    .map(|m| format!("{m:?}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                Err(SshError::AuthFailed {
                    methods: if methods.is_empty() {
                        String::from("сервер не назвал ни одного")
                    } else {
                        methods
                    },
                })
            }
        }
    }

    /// Открыть канал-сессию, переживая мгновенный отказ сервера.
    ///
    /// Слот сессии на сервере освобождается не в момент обмена `close`: sshd сперва
    /// прибирает дочерний процесс, а сигнал о его завершении приходит асинхронно.
    /// Поэтому даже при соблюдении собственного предела (`MAX_CONCURRENT_CHANNELS`)
    /// свежий канал может упереться в отжившую, но ещё не прибранную сессию — сервер
    /// отвечает отказом, хотя место вот-вот освободится. Такой отказ — повод
    /// подождать и повторить, а не ошибка: наблюдалось вживую 2026-08-25, когда
    /// двенадцать очередей через семафор на восемь всё равно поймали
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
                        осталось_попыток = attempts_left,
                        "сервер отказал в открытии канала, ждём и повторяем"
                    );
                    tokio::time::sleep(delay).await;
                    delay = (delay * 2).min(std::time::Duration::from_secs(1));
                }
                Err(e) => return Err(SshError::protocol(e)),
            }
        }
    }

    /// Занять место под канал, дождавшись очереди при необходимости.
    ///
    /// Ожидание здесь правильнее отказа: превышение предела сервера — не ошибка
    /// пользователя и не повод прерывать работу.
    pub(crate) async fn acquire_channel(&self) -> Result<tokio::sync::OwnedSemaphorePermit> {
        self.channels
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| SshError::Protocol(String::from("соединение закрывается")))
    }

    pub fn address(&self) -> &ServerAddress {
        &self.addr
    }

    pub fn user(&self) -> &str {
        &self.user
    }

    /// Живо ли соединение. Дёшево: без обращения к сети.
    pub fn is_alive(&self) -> bool {
        !self.handle.is_closed()
    }

    /// Вежливо разорвать соединение.
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
