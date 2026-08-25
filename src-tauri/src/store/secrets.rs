//! T009 — секреты только в хранилище учётных данных операционной системы.
//!
//! Конституция, принцип IV. Приложение раздаётся людям и держит доступы к их серверам:
//! утечка здесь — это чужой сервер, а не свой. Поэтому пароли, парольные фразы и приватные
//! ключи не пишутся ни в настройки, ни в локальную базу — там лежит только `SecretRef`,
//! ссылка на запись в хранилище ОС.
//!
//! Хранилище своё на каждой платформе: Credential Manager на Windows, Secret Service
//! на Linux. Выбор делается пер-платформенно в `Cargo.toml`, здесь различий нет.
//!
//! **Важное свойство**: всякий секрет, проходящий через этот слой, автоматически попадает
//! в список вырезаемых из вывода (`super::redact`). Так защита от утечки в журнал не зависит
//! от того, вспомнил ли о ней автор конкретной строчки кода.

use super::redact;

/// Имя службы в хранилище ОС. Под ним пользователь увидит записи в системном менеджере.
const SERVICE: &str = "VRCast Studio";

/// Ссылка на секрет. Именно она — а не значение — хранится в базе и пересекает границы слоёв.
///
/// `Debug` печатает только ссылку: это не секрет, а его адрес.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SecretRef(String);

impl SecretRef {
    /// Секрет доступа к серверу: пароль либо парольная фраза ключа.
    pub fn for_server(server_id: &str) -> Self {
        Self(format!("server/{server_id}"))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Восстановить ссылку из значения, прочитанного в базе.
    pub fn from_stored(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

impl std::fmt::Display for SecretRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    #[error("секрет не найден в хранилище операционной системы")]
    NotFound,

    /// Сообщение нижележащей библиотеки проходит через вырезание: она о наших правилах
    /// не знает и вполне может вставить в текст ошибки то, что мы прячем.
    #[error("хранилище учётных данных операционной системы недоступно: {0}")]
    Backend(String),
}

pub type Result<T> = std::result::Result<T, SecretError>;

/// Хранилище секретов.
///
/// Отделено интерфейсом от реализации не ради абстракции как таковой, а чтобы тесты
/// не трогали настоящее хранилище пользователя: тест, оставляющий за собой записи
/// в системном менеджере паролей, — плохой тест.
pub trait SecretStore: Send + Sync {
    fn set(&self, reference: &SecretRef, value: &str) -> Result<()>;
    fn get(&self, reference: &SecretRef) -> Result<String>;
    fn delete(&self, reference: &SecretRef) -> Result<()>;
}

/// Настоящее хранилище операционной системы.
#[derive(Debug, Default)]
pub struct OsSecretStore;

impl OsSecretStore {
    pub fn new() -> Self {
        Self
    }

    fn entry(reference: &SecretRef) -> Result<keyring::Entry> {
        keyring::Entry::new(SERVICE, reference.as_str())
            .map_err(|e| SecretError::Backend(redact::safe_display(&e)))
    }
}

impl SecretStore for OsSecretStore {
    fn set(&self, reference: &SecretRef, value: &str) -> Result<()> {
        // Регистрируем ДО записи: если запись провалится, сообщение об ошибке уже
        // не сможет вынести секрет наружу.
        redact::register(value);

        Self::entry(reference)?
            .set_password(value)
            .map_err(|e| SecretError::Backend(redact::safe_display(&e)))?;
        tracing::debug!(reference = %reference, "секрет сохранён в хранилище ОС");
        Ok(())
    }

    fn get(&self, reference: &SecretRef) -> Result<String> {
        let value = Self::entry(reference)?.get_password().map_err(|e| {
            if matches!(e, keyring::Error::NoEntry) {
                SecretError::NotFound
            } else {
                SecretError::Backend(redact::safe_display(&e))
            }
        })?;
        redact::register(&value);
        Ok(value)
    }

    fn delete(&self, reference: &SecretRef) -> Result<()> {
        // Значение читается ДО удаления, чтобы снять маскировку именно с него.
        // Иначе оно осталось бы в списке вырезаемых навсегда — не беда сама
        // по себе, но список растёт с каждым удалённым профилем, а вырезание
        // проходит по каждой строке журнала.
        let было = Self::entry(reference)
            .ok()
            .and_then(|e| e.get_password().ok());

        match Self::entry(reference)?.delete_credential() {
            Ok(()) => {
                if let Some(value) = было {
                    redact::forget(&value);
                }
                tracing::debug!(reference = %reference, "секрет удалён из хранилища ОС");
                Ok(())
            }
            // Удаление отсутствующего — не ошибка: повтор обязан быть безопасным
            // (конституция, принцип V).
            Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(SecretError::Backend(redact::safe_display(&e))),
        }
    }
}

/// Хранилище в памяти — для тестов. Настоящее хранилище пользователя не трогает.
#[derive(Debug, Default)]
pub struct InMemorySecretStore {
    items: std::sync::RwLock<std::collections::HashMap<String, String>>,
}

impl InMemorySecretStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl SecretStore for InMemorySecretStore {
    fn set(&self, reference: &SecretRef, value: &str) -> Result<()> {
        redact::register(value);
        self.items
            .write()
            .map_err(|_| SecretError::Backend("хранилище в памяти повреждено".into()))?
            .insert(reference.as_str().to_owned(), value.to_owned());
        Ok(())
    }

    fn get(&self, reference: &SecretRef) -> Result<String> {
        let value = self
            .items
            .read()
            .map_err(|_| SecretError::Backend("хранилище в памяти повреждено".into()))?
            .get(reference.as_str())
            .cloned()
            .ok_or(SecretError::NotFound)?;
        redact::register(&value);
        Ok(value)
    }

    fn delete(&self, reference: &SecretRef) -> Result<()> {
        let было = self
            .items
            .write()
            .map_err(|_| SecretError::Backend("хранилище в памяти повреждено".into()))?
            .remove(reference.as_str());
        // Снимаем маскировку с ИМЕННО ЭТОГО значения, а не со всех сразу:
        // у остальных профилей секреты живы (T073).
        if let Some(value) = было {
            redact::forget(&value);
        }
        Ok(())
    }
}
