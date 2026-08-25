//! Общая оснастка договорных тестов.
//!
//! Состояние приложения собирается с базой в памяти и хранилищем секретов в памяти:
//! тест, оставляющий за собой записи в системном менеджере паролей пользователя, —
//! плохой тест.

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
    .expect("состояние приложения не собралось")
}

/// Заведомо годные поля профиля. Тесты меняют то, что проверяют, остальное берут отсюда.
pub fn valid_input(name: &str) -> ServerInput {
    ServerInput {
        name: name.to_owned(),
        // Адрес из блока, отведённого под примеры в документации: он не ведёт
        // ни на чей настоящий сервер.
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
