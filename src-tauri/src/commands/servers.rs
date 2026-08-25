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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

/// Предложение перенести настройки из `server.env` (T043).
#[derive(Debug, Clone, Serialize)]
pub struct ImportSuggestion {
    /// Откуда взято — человек должен понимать, что ему подставили и откуда.
    pub source: String,
    /// Ключ есть, а парольной фразы в файле нет: её придётся ввести.
    pub needs_passphrase: bool,
    pub input: ServerInput,
}

/// Собрать профиль из присланных полей.
///
/// Номер, ссылка на секрет и отметка активности не приходят снаружи: их ставит ядро.
fn profile_from(input: ServerInput, id: String, secret_ref: String) -> ServerProfile {
    let mut p = ServerProfile::new(id, input.name);
    p.host = input.host;
    p.port = input.port;
    p.user = input.user;
    p.auth_kind = input.auth_kind;
    p.key_path = input.key_path;
    p.domain = input.domain;
    p.video_dir = input
        .video_dir
        .filter(|d| !d.trim().is_empty())
        .unwrap_or_else(|| String::from(crate::domain::server_profile::DEFAULT_VIDEO_DIR));
    p.cdn_base = input.cdn_base;
    p.ipv6_mode = input.ipv6_mode;
    p.secret_ref = secret_ref;
    p
}

/// Проверить профиль и превратить замечания в ошибку договора.
///
/// Замечания склеиваются в одно сообщение, а не теряются: их бывает несколько,
/// и человеку нужно увидеть все сразу, а не по одному за круг.
fn check(profile: &ServerProfile) -> Result<()> {
    profile.validate().map_err(|problems| {
        let text = problems
            .iter()
            .map(|p| p.message.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        AppError::new(ErrorCode::InvalidInput)
            .with_message(text)
            .with_cause(
                problems
                    .iter()
                    .map(|p| p.field)
                    .collect::<Vec<_>>()
                    .join(", "),
            )
    })
}

/// Отказ на ссылку в пустоту. Отдельная функция — чтобы формулировка была одна
/// на все команды: пользователь не должен гадать, одно ли это и то же.
pub(crate) fn no_such_server(id: &str) -> AppError {
    AppError::new(ErrorCode::InvalidInput)
        .with_message("Такого сервера нет — возможно, его профиль удалили.")
        .with_cause(id)
}

pub mod api {
    use super::*;
    use crate::store::profiles;
    use crate::store::secrets::SecretRef;

    /// Список профилей. Без секретов — их здесь физически нет.
    pub fn servers_list(state: &AppState) -> Result<Vec<ServerProfile>> {
        Ok(profiles::list(&state.db)?)
    }

    /// Добавить профиль. Секрет уходит в хранилище ОС, в профиль пишется только ссылка.
    pub fn server_add(state: &AppState, input: ServerInput, secret: &str) -> Result<String> {
        let id = format!("srv_{}", uuid::Uuid::new_v4().simple());
        let reference = SecretRef::for_server(&id);

        let mut profile = profile_from(input, id.clone(), reference.as_str().to_owned());
        profile.normalize();
        check(&profile)?;

        if profiles::name_taken(&state.db, &profile.name, None)? {
            return Err(AppError::new(ErrorCode::InvalidInput)
                .with_message("Профиль с таким названием уже есть — выберите другое.")
                .with_cause(&profile.name));
        }

        profiles::insert(&state.db, &profile)?;

        // Секрет — после записи профиля, и с уборкой при неудаче: иначе в системном
        // менеджере паролей осталась бы запись, на которую ничто не ссылается,
        // и удалить её пользователю было бы нечем.
        if let Err(e) = state.secrets.set(&reference, secret) {
            let _ = profiles::remove(&state.db, &id);
            return Err(e.into());
        }

        tracing::info!(server = %id, "профиль сервера создан");
        Ok(id)
    }

    /// Изменить профиль. Секрет заменяется, **только если передан**: иначе изменение
    /// имени профиля стирало бы пароль, и узнал бы об этом пользователь только
    /// при следующем подключении.
    pub fn server_update(
        state: &AppState,
        id: &str,
        input: ServerInput,
        secret: Option<&str>,
    ) -> Result<()> {
        let existing = profiles::get(&state.db, id)?.ok_or_else(|| no_such_server(id))?;

        let mut profile = profile_from(input, existing.id.clone(), existing.secret_ref.clone());
        // Отметку активности и подтверждённый отпечаток правка полей не трогает:
        // и то и другое — отдельные осознанные действия пользователя.
        profile.is_active = existing.is_active;
        profile.host_fingerprint = existing.host_fingerprint.clone();
        profile.normalize();
        check(&profile)?;

        if profiles::name_taken(&state.db, &profile.name, Some(id))? {
            return Err(AppError::new(ErrorCode::InvalidInput)
                .with_message("Профиль с таким названием уже есть — выберите другое.")
                .with_cause(&profile.name));
        }

        profiles::update(&state.db, &profile)?;

        if let Some(value) = secret {
            state
                .secrets
                .set(&SecretRef::from_stored(&profile.secret_ref), value)?;
        }
        Ok(())
    }

    /// Удалить профиль вместе с записью секрета в хранилище ОС.
    ///
    /// Оставленный секрет — это доступ к чужому серверу, о котором пользователь
    /// уже не помнит (FR-005).
    pub fn server_remove(state: &AppState, id: &str) -> Result<()> {
        // Отсутствие профиля не ошибка: повтор обязан быть безопасным
        // (договор, правило 5).
        let Some(profile) = profiles::get(&state.db, id)? else {
            return Ok(());
        };

        profiles::remove(&state.db, id)?;
        if let Err(e) = state
            .secrets
            .delete(&SecretRef::from_stored(&profile.secret_ref))
        {
            // Профиль уже удалён. Сообщить о повисшем секрете важнее, чем промолчать,
            // но возвращать ошибку нельзя: тогда повторное удаление стало бы
            // невозможным, а профиля уже нет.
            tracing::error!(server = %id, error = %e, "секрет удалённого профиля остался в хранилище");
        }
        tracing::info!(server = %id, "профиль сервера удалён");
        Ok(())
    }

    /// Сделать профиль активным. Активен ровно один (FR-002).
    pub fn server_set_active(state: &AppState, id: &str) -> Result<()> {
        if profiles::set_active(&state.db, id)? {
            Ok(())
        } else {
            Err(no_such_server(id))
        }
    }

    /// Пошаговая проверка подключения (FR-003).
    ///
    /// Возвращает **все** шаги, а не только сломавшийся: пользователю нужно видеть,
    /// что успело пройти. Ошибкой команда не завершается — неудача шага это данные,
    /// а не отказ команды.
    pub async fn server_test(state: &AppState, id: &str) -> Result<Vec<TestStep>> {
        let profile = profiles::get(&state.db, id)?.ok_or_else(|| no_such_server(id))?;
        Ok(super::probe::run(state, &profile).await)
    }

    /// Предложить перенос настроек из `server.env` (T043).
    ///
    /// Отсутствие файла — не ошибка, а обычное дело: у большинства пользователей
    /// приложения его нет и не будет. Возвращается `None`, и мастер просто
    /// не показывает это предложение.
    ///
    /// Ничего не создаётся и не записывается: это только заполнение формы, которую
    /// человек увидит и сможет поправить.
    pub fn server_import_suggestion(state: &AppState) -> Result<Option<ImportSuggestion>> {
        // Предложение имеет смысл только для первого профиля: дальше пользователь
        // заводит серверы сам, и подставлять ему один и тот же файл незачем.
        if !profiles::list(&state.db)?.is_empty() {
            return Ok(None);
        }

        Ok(crate::server::env_import::default_location()
            .and_then(|path| crate::server::env_import::read_from(&path))
            .map(|imported| ImportSuggestion {
                source: imported.source.to_string_lossy().into_owned(),
                needs_passphrase: imported.needs_passphrase,
                input: imported.input,
            }))
    }

    /// Подтвердить отпечаток сервера (FR-092).
    pub fn server_fingerprint_confirm(state: &AppState, id: &str, fingerprint: &str) -> Result<()> {
        let fingerprint = fingerprint.trim();
        if fingerprint.is_empty() {
            return Err(AppError::new(ErrorCode::InvalidInput)
                .with_message("Отпечаток пуст — подтверждать нечего."));
        }
        if profiles::set_fingerprint(&state.db, id, fingerprint)? {
            tracing::info!(server = %id, "отпечаток сервера подтверждён пользователем");
            Ok(())
        } else {
            Err(no_such_server(id))
        }
    }
}

/// Тонкие обёртки для оболочки. Логики здесь нет — только вызов `api`.
pub mod ipc {
    use super::*;
    use tauri::State;

    #[tauri::command]
    pub fn servers_list(state: State<'_, AppState>) -> Result<Vec<ServerProfile>> {
        api::servers_list(&state)
    }

    #[tauri::command]
    pub fn server_add(
        state: State<'_, AppState>,
        input: ServerInput,
        secret: String,
    ) -> Result<String> {
        api::server_add(&state, input, &secret)
    }

    #[tauri::command]
    pub fn server_update(
        state: State<'_, AppState>,
        id: String,
        input: ServerInput,
        secret: Option<String>,
    ) -> Result<()> {
        api::server_update(&state, &id, input, secret.as_deref())
    }

    #[tauri::command]
    pub fn server_remove(state: State<'_, AppState>, id: String) -> Result<()> {
        api::server_remove(&state, &id)
    }

    #[tauri::command]
    pub fn server_set_active(state: State<'_, AppState>, id: String) -> Result<()> {
        api::server_set_active(&state, &id)
    }

    #[tauri::command]
    pub async fn server_test(state: State<'_, AppState>, id: String) -> Result<Vec<TestStep>> {
        api::server_test(&state, &id).await
    }

    #[tauri::command]
    pub fn server_fingerprint_confirm(
        state: State<'_, AppState>,
        id: String,
        fingerprint: String,
    ) -> Result<()> {
        api::server_fingerprint_confirm(&state, &id, &fingerprint)
    }

    #[tauri::command]
    pub fn server_import_suggestion(
        state: State<'_, AppState>,
    ) -> Result<Option<ImportSuggestion>> {
        api::server_import_suggestion(&state)
    }
}

/// T041 — пошаговая проверка подключения.
mod probe {
    use super::*;
    use crate::domain::server_profile::AuthKind;
    use crate::ssh::{Connection, Credentials, ServerAddress};
    use crate::store::secrets::SecretRef;
    use std::time::Duration;

    /// Сколько ждём отклика на каждом шаге.
    ///
    /// Проверка идёт по нажатию кнопки, и человек смотрит на неё. Полминуты молчания
    /// он воспримет как зависшее приложение, а не как медленный сервер.
    const STEP_TIMEOUT: Duration = Duration::from_secs(10);

    fn step(index: usize, status: StepStatus, detail: Option<String>) -> TestStep {
        let (id, title) = TEST_STEPS[index];
        TestStep {
            id: id.to_owned(),
            title: title.to_owned(),
            status,
            detail,
        }
    }

    /// Достроить остаток шагов как невыполнявшиеся.
    ///
    /// Именно это отличает отчёт о проверке от сообщения об ошибке: человек видит
    /// не «что-то не так», а «прошло вот это, оборвалось здесь, дальше не смотрели».
    fn skip_rest(steps: &mut Vec<TestStep>) {
        while steps.len() < TEST_STEPS.len() {
            steps.push(step(steps.len(), StepStatus::Skipped, None));
        }
    }

    pub async fn run(state: &AppState, profile: &ServerProfile) -> Vec<TestStep> {
        let mut steps: Vec<TestStep> = Vec::with_capacity(TEST_STEPS.len());

        // 1. Сеть.
        let addr = format!("{}:{}", profile.host, profile.port);
        match tokio::time::timeout(STEP_TIMEOUT, tokio::net::TcpStream::connect(&addr)).await {
            Ok(Ok(_)) => steps.push(step(
                0,
                StepStatus::Ok,
                Some(format!("порт {} отвечает", profile.port)),
            )),
            Ok(Err(e)) => {
                steps.push(step(0, StepStatus::Failed, Some(e.to_string())));
                skip_rest(&mut steps);
                return steps;
            }
            Err(_) => {
                steps.push(step(
                    0,
                    StepStatus::Failed,
                    Some(format!("сервер не ответил за {} с", STEP_TIMEOUT.as_secs())),
                ));
                skip_rest(&mut steps);
                return steps;
            }
        }

        // 2. Вход. Учётные данные не отправляются серверу, отпечаток которого
        // не подтверждён, — поэтому неподтверждённый отпечаток останавливает
        // проверку здесь, а не превращается в загадочную неудачу входа.
        let Some(expected) = profile.host_fingerprint.clone() else {
            steps.push(step(
                1,
                StepStatus::Failed,
                Some(String::from(
                    "отпечаток сервера ещё не подтверждён — подтвердите его, и проверка пойдёт дальше",
                )),
            ));
            skip_rest(&mut steps);
            return steps;
        };

        let secret = match state
            .secrets
            .get(&SecretRef::from_stored(&profile.secret_ref))
        {
            Ok(s) => s,
            Err(e) => {
                steps.push(step(1, StepStatus::Failed, Some(e.to_string())));
                skip_rest(&mut steps);
                return steps;
            }
        };

        let credentials = match profile.auth_kind {
            AuthKind::Key => Credentials::Key {
                path: profile.key_path.clone().unwrap_or_default().into(),
                passphrase: Some(secret),
            },
            AuthKind::Password => Credentials::Password(secret),
        };

        let conn = match Connection::connect(
            ServerAddress::new(&profile.host, profile.port),
            &profile.user,
            credentials,
            &expected,
        )
        .await
        {
            Ok(c) => {
                steps.push(step(
                    1,
                    StepStatus::Ok,
                    Some(format!("вошли как {}", profile.user)),
                ));
                c
            }
            Err(e) => {
                // Подробность проходит вырезание секретов: она приходит от чужой
                // библиотеки, которая о наших правилах ничего не знает.
                steps.push(step(
                    1,
                    StepStatus::Failed,
                    Some(crate::store::redact::safe_display(&e)),
                ));
                skip_rest(&mut steps);
                return steps;
            }
        };

        // 3. Каталог с видео. Проверяем и чтение, и запись: узнать о нехватке прав
        // на первой же заливке — поздно, файл к тому времени уже качается.
        let probe_cmd = format!(
            "test -d '{dir}' && test -r '{dir}' && test -w '{dir}'",
            dir = profile.video_dir
        );
        match conn.exec(&probe_cmd).await {
            Ok(out) if out.ok() => steps.push(step(
                2,
                StepStatus::Ok,
                Some(format!("{} доступен на чтение и запись", profile.video_dir)),
            )),
            Ok(_) => {
                steps.push(step(
                    2,
                    StepStatus::Failed,
                    Some(format!(
                        "каталога {} нет либо у пользователя {} нет прав на него",
                        profile.video_dir, profile.user
                    )),
                ));
                skip_rest(&mut steps);
                conn.close().await;
                return steps;
            }
            Err(e) => {
                steps.push(step(
                    2,
                    StepStatus::Failed,
                    Some(crate::store::redact::safe_display(&e)),
                ));
                skip_rest(&mut steps);
                conn.close().await;
                return steps;
            }
        }
        conn.close().await;

        // 4. Отдача по домену.
        steps.push(check_domain(&profile.domain).await);
        steps
    }

    /// Проверить, что раздача отвечает по домену — **с машины пользователя**.
    ///
    /// Проверять изнутри сервера бессмысленно: оттуда «работает» и то, что снаружи
    /// недоступно из-за доменной записи или сетевого фильтра.
    ///
    /// Что этот шаг доказывает: домен разрешается, ведёт сюда, сертификат для него
    /// действителен, веб-сервер отвечает. Чего не доказывает: что раздаётся именно
    /// ожидаемое содержимое. Для этого нужен известный файл, и такая проверка —
    /// шаг `verify` при развёртывании (R-20, FR-125).
    async fn check_domain(domain: &str) -> TestStep {
        let url = format!("https://{domain}/{}/", crate::domain::links::VIDEOS_PREFIX);

        let client = match reqwest::Client::builder().timeout(STEP_TIMEOUT).build() {
            Ok(c) => c,
            Err(e) => return step(3, StepStatus::Failed, Some(e.to_string())),
        };

        match client.get(&url).send().await {
            Ok(response) => {
                // Ответ веб-сервера — уже успех: каталог может быть закрыт для
                // перечисления, и это правильная его настройка, а не поломка.
                let code = response.status().as_u16();
                step(
                    3,
                    StepStatus::Ok,
                    Some(format!("{domain} отвечает по HTTPS (код {code})")),
                )
            }
            Err(e) => {
                let detail = if e.is_timeout() {
                    format!("{domain} не ответил за {} с", STEP_TIMEOUT.as_secs())
                } else if e.is_connect() {
                    format!(
                        "не удалось соединиться с {domain}: проверьте, что доменная запись ведёт на этот сервер"
                    )
                } else {
                    crate::store::redact::safe_display(&e)
                };
                step(3, StepStatus::Failed, Some(detail))
            }
        }
    }
}
