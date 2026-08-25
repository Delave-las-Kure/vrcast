//! T040–T042 — команды управления профилями серверов.
//!
//! Договор: `contracts/ipc-commands.md`, раздел «Серверы».
//!
//! Главное правило этого раздела: **секрет пересекает границу ровно один раз** —
//! когда интерфейс передаёт его при создании или изменении профиля. Обратно секрет
//! не возвращается никогда, ни одной командой (FR-090, FR-091). Поэтому у ответов
//! здесь нет и не может быть поля под секрет: возвращается `ServerProfile`, в котором
//! лежит только ссылка на запись в хранилище ОС.

use super::error::{AppError, ErrorCode, Result};
use super::AppState;
use crate::domain::server_profile::{AuthKind, Ipv6Mode, ServerProfile};
use serde::{Deserialize, Serialize};

/// Поля профиля в том виде, в каком их присылает интерфейс.
///
/// Отдельный тип от [`ServerProfile`] намеренно: интерфейс не задаёт ни номер
/// профиля, ни ссылку на секрет, ни отпечаток — их проставляет ядро. Приняв
/// профиль целиком, мы дали бы интерфейсу возможность подменить ссылку на секрет
/// чужого профиля.
#[derive(Debug, Clone, Deserialize)]
pub struct ServerInput {
    pub name: String,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub auth_kind: AuthKind,
    /// Только при входе по ключу.
    pub key_path: Option<String>,
    pub domain: String,
    /// Пусто = каталог раздачи по умолчанию.
    pub video_dir: Option<String>,
    pub cdn_base: Option<String>,
    pub ipv6_mode: Option<Ipv6Mode>,
}

/// Как прошёл отдельный шаг проверки подключения.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    Ok,
    Failed,
    /// Шаг не выполнялся: остановились раньше. Показывается наравне с остальными —
    /// пользователю важно видеть, где именно оборвалось (FR-003).
    Skipped,
}

/// Один шаг проверки подключения.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestStep {
    /// Устойчивое имя шага: `network`, `login`, `video_dir`, `domain`.
    pub id: String,
    /// Название для показа.
    pub title: String,
    pub status: StepStatus,
    /// Подробность: что именно ответил сервер, чего не хватило.
    pub detail: Option<String>,
}

/// Порядок шагов проверки. Он же — порядок показа.
///
/// Порядок не произволен: каждый следующий шаг имеет смысл только при успехе
/// предыдущего. Проверять отдачу по домену, не сумев войти на сервер, — значит
/// сообщить пользователю вторую беду, не назвав первую.
pub const TEST_STEPS: [(&str, &str); 4] = [
    ("network", "Сервер доступен по сети"),
    ("login", "Вход на сервер"),
    ("video_dir", "Каталог с видео доступен"),
    ("domain", "Раздача отвечает по домену"),
];

pub mod api {
    use super::*;

    /// Список профилей. Без секретов — их здесь физически нет.
    pub fn servers_list(_state: &AppState) -> Result<Vec<ServerProfile>> {
        Err(not_implemented("servers_list"))
    }

    /// Добавить профиль. Секрет уходит в хранилище ОС, в профиль пишется только ссылка.
    pub fn server_add(_state: &AppState, _input: ServerInput, _secret: &str) -> Result<String> {
        Err(not_implemented("server_add"))
    }

    /// Изменить профиль. Секрет заменяется, **только если передан**: иначе изменение
    /// имени профиля стирало бы пароль.
    pub fn server_update(
        _state: &AppState,
        _id: &str,
        _input: ServerInput,
        _secret: Option<&str>,
    ) -> Result<()> {
        Err(not_implemented("server_update"))
    }

    /// Удалить профиль вместе с записью секрета в хранилище ОС.
    pub fn server_remove(_state: &AppState, _id: &str) -> Result<()> {
        Err(not_implemented("server_remove"))
    }

    /// Сделать профиль активным. Активен ровно один (FR-002).
    pub fn server_set_active(_state: &AppState, _id: &str) -> Result<()> {
        Err(not_implemented("server_set_active"))
    }

    /// Пошаговая проверка подключения (FR-003).
    ///
    /// Возвращает **все** шаги, а не только сломавшийся: пользователю нужно видеть,
    /// что успело пройти. Ошибкой команда не завершается — неудача шага это данные,
    /// а не отказ команды.
    pub async fn server_test(_state: &AppState, _id: &str) -> Result<Vec<TestStep>> {
        Err(not_implemented("server_test"))
    }

    /// Подтвердить отпечаток сервера (FR-092).
    pub fn server_fingerprint_confirm(
        _state: &AppState,
        _id: &str,
        _fingerprint: &str,
    ) -> Result<()> {
        Err(not_implemented("server_fingerprint_confirm"))
    }
}

/// Заглушка на время, пока команда не написана.
///
/// Существует ровно до реализации: договорные тесты написаны раньше её и обязаны
/// падать, пока команды нет. Молчаливый успех вместо этого создал бы видимость
/// работающего договора.
fn not_implemented(what: &str) -> AppError {
    AppError::new(ErrorCode::Internal).with_cause(format!("{what}: ещё не реализовано"))
}
