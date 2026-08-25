//! Работа с сервером: то, что нельзя проверить без него.
//!
//! Черта между этим слоем и `domain` проходит по одному признаку: там — **что
//! известно**, здесь — **как это узнать**. Правила разбора описи живут в
//! `domain::manifest` и проверяются без сервера; порядок чтения и записи описи —
//! здесь, и проверяется против настоящего OpenSSH.

pub mod active_use;
pub mod checksum;
pub mod disk;
pub mod env_import;
pub mod free_space;
pub mod listing;
pub mod manifest_io;
pub mod probe_moov;
pub mod reconcile;
pub mod upload;

/// Записи каталога раздачи, которые не являются видео и не показываются в библиотеке.
///
/// Обе — во владении приложения (`contracts/server-contract.md`). Показать их
/// пользователю значило бы предложить ему удалить опись собственной библиотеки.
pub const SERVICE_ENTRIES: [&str; 2] = [manifest_io::MANIFEST_NAME, "_slow"];

/// Заключить строку в кавычки для команды на сервере.
///
/// Пути приходят из профиля пользователя и содержат что угодно: пробелы, кириллицу,
/// изредка кавычки. Подставлять их в команду как есть — это и сломанные пути,
/// и возможность выполнить на сервере не то, что задумано.
pub(crate) fn shell_quote(value: &str) -> String {
    // Внутри одинарных кавычек оболочка не толкует ничего, кроме самой кавычки;
    // её закрывают, экранируют и открывают снова.
    format!("'{}'", value.replace('\'', r"'\''"))
}

/// Соединить каталог и имя в путь на сервере.
pub(crate) fn join_remote(dir: &str, name: &str) -> String {
    format!("{}/{}", dir.trim_end_matches('/'), name)
}

/// Подключиться к серверу по его профилю.
///
/// Одна точка на всё приложение: подключение — единственное место, где секрет
/// достаётся из хранилища, и разводить это по нескольким местам значит однажды
/// забыть где-нибудь про проверку отпечатка.
pub async fn connect(
    secrets: &dyn crate::store::secrets::SecretStore,
    profile: &crate::domain::server_profile::ServerProfile,
) -> crate::ssh::Result<crate::ssh::Connection> {
    use crate::domain::server_profile::AuthKind;
    use crate::ssh::{Connection, Credentials, ServerAddress, SshError};
    use crate::store::secrets::SecretRef;

    let addr = ServerAddress::new(&profile.host, profile.port);

    // Учётные данные не отправляются серверу, отпечаток которого не подтверждён.
    // Здесь это не «строгая настройка», а условие: подтверждения нет — подключения нет.
    let Some(expected) = profile.host_fingerprint.clone() else {
        return Err(SshError::HostKeyUnconfirmed { addr });
    };

    let secret = secrets
        .get(&SecretRef::from_stored(&profile.secret_ref))
        .map_err(|e| SshError::KeyUnreadable {
            path: profile.secret_ref.clone(),
            reason: e.to_string(),
        })?;

    let credentials = match profile.auth_kind {
        AuthKind::Key => Credentials::Key {
            path: profile.key_path.clone().unwrap_or_default().into(),
            passphrase: Some(secret),
        },
        AuthKind::Password => Credentials::Password(secret),
    };

    Connection::connect(addr, &profile.user, credentials, &expected).await
}
