//! T023 — способы входа на сервер, включая ключ с парольной фразой (FR-096).

use super::{Result, SshError};
use crate::store::redact;
use russh::keys::PrivateKey;
use std::path::{Path, PathBuf};

/// Чем входим на сервер.
///
/// Секрет здесь живёт ровно столько, сколько идёт подключение: в базу он не попадает,
/// а берётся из хранилища ОС по ссылке (конституция, принцип IV).
#[derive(Clone)]
pub enum Credentials {
    /// Приватный ключ. Парольная фраза нужна не всегда — но если ключ ею защищён,
    /// без неё он не прочитается, и это отдельная, понятная пользователю ошибка.
    Key {
        path: PathBuf,
        passphrase: Option<String>,
    },
    Password(String),
}

/// `Debug` намеренно не печатает ни пароль, ни парольную фразу: структура может попасть
/// в отладочный вывод целиком, и это самый частый способ утечки.
impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Key { path, passphrase } => f
                .debug_struct("Credentials::Key")
                .field("path", path)
                .field(
                    "passphrase",
                    &if passphrase.is_some() {
                        "задана"
                    } else {
                        "нет"
                    },
                )
                .finish(),
            Self::Password(_) => f.write_str("Credentials::Password(<скрыт>)"),
        }
    }
}

/// Прочитать приватный ключ с диска.
///
/// Различает «ключ защищён парольной фразой» и «ключ не читается» — это разные причины
/// и разные подсказки пользователю (FR-105). Слить их в одну ошибку значило бы заставить
/// человека гадать, не тот ли он файл выбрал.
pub fn load_key(path: &Path, passphrase: Option<&str>) -> Result<PrivateKey> {
    // Регистрируем парольную фразу до чтения: если чтение провалится, сообщение об ошибке
    // уже не сможет вынести её наружу.
    if let Some(p) = passphrase {
        redact::register(p);
    }

    russh::keys::load_secret_key(path, passphrase).map_err(|e| {
        let shown = path.display().to_string();
        match e {
            russh::keys::Error::KeyIsEncrypted => SshError::KeyNeedsPassphrase { path: shown },
            other => SshError::KeyUnreadable {
                path: shown,
                reason: redact::safe_display(&other),
            },
        }
    })
}
