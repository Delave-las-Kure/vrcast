//! T040–T042 — команды управления профилями серверов.
//!
//! Договор: `contracts/ipc-commands.md`, раздел «Серверы».
//!
//! Главное правило этого раздела: **секрет пересекает границу ровно один раз** —
//! когда интерфейс передаёт его при создании или изменении профиля. Обратно секрет
//! не возвращается никогда, ни одной командой (FR-090, FR-091). Поэтому у ответов
//! здесь нет и не может быть поля под секрет: возвращается `ServerProfile`, в котором
//! лежит только ссылка на запись в хранилище ОС.

use super::error::{AppError, DetailCode, ErrorCode, Result};
use super::AppState;
use crate::domain::server_profile::{AuthKind, Ipv6Mode, ServerProfile};
use crate::domain::wording::Detail;
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
    /// Stable step name: `network`, `login`, `video_dir`, `domain`. The interface
    /// looks up its title by this, so the title no longer travels with every step.
    pub id: String,
    pub status: StepStatus,
    /// What to say about the outcome: what the server answered, what was missing.
    pub detail: Option<Detail>,
}

/// Порядок шагов проверки. Он же — порядок показа.
///
/// Порядок не произволен: каждый следующий шаг имеет смысл только при успехе
/// предыдущего. Проверять отдачу по домену, не сумев войти на сервер, — значит
/// сообщить пользователю вторую беду, не назвав первую.
pub const TEST_STEPS: [&str; 4] = ["network", "login", "video_dir", "domain"];

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
        AppError::new(ErrorCode::InvalidInput)
            .with_details(problems.iter().map(|p| p.detail.clone()))
            // The fields are named in the particulars: the interface highlights them,
            // and a support log should say which ones were wrong without the wording.
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
        .detail(DetailCode::ProfileNotFound)
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
                .with_detail(
                    Detail::new(DetailCode::ProfileNameTaken).with("name", profile.name.clone()),
                )
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
                .with_detail(
                    Detail::new(DetailCode::ProfileNameTaken).with("name", profile.name.clone()),
                )
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
            return Err(AppError::new(ErrorCode::InvalidInput).detail(DetailCode::FingerprintEmpty));
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

    fn step(index: usize, status: StepStatus, detail: Option<Detail>) -> TestStep {
        TestStep {
            id: TEST_STEPS[index].to_owned(),
            status,
            detail,
        }
    }

    /// A step that went well, or did not, with one thing to say about it.
    fn said(index: usize, status: StepStatus, detail: Detail) -> TestStep {
        step(index, status, Some(detail))
    }

    /// A complaint from a library, kept in its own words.
    fn system(e: impl std::fmt::Display) -> Detail {
        Detail::new(DetailCode::SystemError).with("text", crate::store::redact::safe_display(&e))
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
        match reach_ssh(&profile.host, profile.port).await {
            Ok(banner) => steps.push(said(0, StepStatus::Ok, banner)),
            Err(detail) => {
                steps.push(said(0, StepStatus::Failed, detail));
                skip_rest(&mut steps);
                return steps;
            }
        }

        // 2. Вход. Учётные данные не отправляются серверу, отпечаток которого
        // не подтверждён, — поэтому неподтверждённый отпечаток останавливает
        // проверку здесь, а не превращается в загадочную неудачу входа.
        let Some(expected) = profile.host_fingerprint.clone() else {
            steps.push(said(
                1,
                StepStatus::Failed,
                Detail::new(DetailCode::StepLoginFingerprintUnconfirmed),
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
                steps.push(said(1, StepStatus::Failed, system(e)));
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
                steps.push(said(
                    1,
                    StepStatus::Ok,
                    Detail::new(DetailCode::StepLoginOk).with("user", profile.user.clone()),
                ));
                c
            }
            Err(e) => {
                // Подробность проходит вырезание секретов: она приходит от чужой
                // библиотеки, которая о наших правилах ничего не знает.
                steps.push(said(1, StepStatus::Failed, system(&e)));
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
            Ok(out) if out.ok() => steps.push(said(
                2,
                StepStatus::Ok,
                Detail::new(DetailCode::StepVideoDirOk).with("dir", profile.video_dir.clone()),
            )),
            Ok(_) => {
                steps.push(said(
                    2,
                    StepStatus::Failed,
                    Detail::new(DetailCode::StepVideoDirMissingOrDenied)
                        .with("dir", profile.video_dir.clone())
                        .with("user", profile.user.clone()),
                ));
                skip_rest(&mut steps);
                conn.close().await;
                return steps;
            }
            Err(e) => {
                steps.push(said(2, StepStatus::Failed, system(&e)));
                skip_rest(&mut steps);
                conn.close().await;
                return steps;
            }
        }

        // Берём имя настоящего файла из каталога — им и будем проверять отдачу.
        // Это и есть разница между «веб-сервер отвечает» и «раздача работает».
        let sample = sample_file(&conn, &profile.video_dir).await;
        conn.close().await;

        // 4. Отдача по домену.
        steps.push(check_domain(&profile.domain, sample.as_deref()).await);
        steps
    }

    /// Имя любого видеофайла из каталога раздачи.
    ///
    /// Нужно, чтобы проверить отдачу настоящим файлом, а не корнем каталога.
    /// Отсутствие файлов — не беда: на свежем сервере их и нет, проверка тогда
    /// сделает что может и честно скажет, чего не проверяла.
    async fn sample_file(conn: &Connection, video_dir: &str) -> Option<String> {
        let cmd = format!(
            "find {} -maxdepth 1 -type f -name '*.mp4' -printf '%f\\n' 2>/dev/null | head -n 1",
            crate::server::shell_quote(video_dir)
        );
        let out = conn.exec(&cmd).await.ok()?;
        let name = out.trimmed().trim().to_owned();
        if name.is_empty() {
            None
        } else {
            Some(name)
        }
    }

    /// Достучаться до SSH и убедиться, что там **именно он**.
    ///
    /// Установленного соединения мало. У части хостеров перед сервером стоит защита
    /// от атак, которая сама завершает рукопожатие TCP на **любом** порту — и молчит.
    /// Проверено на боевом сервере автора 2026-08-25: порты 64999, 12345 и 54321
    /// «отвечали» ровно так же, как 22, хотя за ними нет ничего.
    ///
    /// Поэтому шаг считается пройденным, только если сервер представился: настоящий
    /// SSH шлёт строку `SSH-2.0-…` сразу после соединения, ничего не дожидаясь.
    /// Это тот же принцип, что и в проверке отдачи по домену (R-20): открытый порт
    /// не доказывает ничего, доказывает ответ.
    async fn reach_ssh(host: &str, port: u16) -> std::result::Result<Detail, Detail> {
        use tokio::io::AsyncReadExt;

        let addr = format!("{host}:{port}");
        let mut stream =
            match tokio::time::timeout(STEP_TIMEOUT, tokio::net::TcpStream::connect(&addr)).await {
                Ok(Ok(s)) => s,
                Ok(Err(e)) => return Err(system(e)),
                Err(_) => {
                    return Err(Detail::new(DetailCode::StepNetTimeout)
                        .with("seconds", STEP_TIMEOUT.as_secs()))
                }
            };

        // Баннера хватает первых нескольких десятков байт; ждать его долго незачем —
        // настоящий SSH шлёт его немедленно.
        let mut buf = [0u8; 128];
        let read = tokio::time::timeout(STEP_TIMEOUT, stream.read(&mut buf)).await;

        let bytes = match read {
            Ok(Ok(0)) => return Err(Detail::new(DetailCode::StepNetClosed)),
            Ok(Ok(n)) => &buf[..n],
            Ok(Err(e)) => return Err(system(e)),
            Err(_) => return Err(Detail::new(DetailCode::StepNetSilent)),
        };

        let banner = String::from_utf8_lossy(bytes);
        let first_line = banner.lines().next().unwrap_or("").trim();
        if first_line.starts_with("SSH-") {
            Ok(Detail::new(DetailCode::StepNetBanner).with("banner", first_line.to_owned()))
        } else {
            Err(Detail::new(DetailCode::StepNetNotSsh)
                .with("port", port)
                .with("got", first_line.chars().take(40).collect::<String>()))
        }
    }

    /// Проверить, что раздача отвечает по домену — **с машины пользователя**.
    ///
    /// Проверять изнутри сервера бессмысленно: оттуда «работает» и то, что снаружи
    /// недоступно из-за доменной записи или сетевого фильтра.
    ///
    /// Что этот шаг доказывает, когда в каталоге есть хотя бы один файл: домен
    /// разрешается, ведёт сюда, сертификат действителен, **и раздача действительно
    /// отдаёт содержимое этого файла**. Ради последнего запрашивается ровно один
    /// байт настоящего файла: без него проверка сводилась бы к «веб-сервер отвечает»,
    /// а это, как и открытый порт, не доказывает ничего (R-20).
    ///
    /// Когда файлов нет, проверяется только доступность домена — и об этом сказано
    /// в подробности шага, чтобы успех не выглядел полнее, чем он есть.
    async fn check_domain(domain: &str, sample: Option<&str>) -> TestStep {
        let client = match reqwest::Client::builder().timeout(STEP_TIMEOUT).build() {
            Ok(c) => c,
            Err(e) => return said(3, StepStatus::Failed, system(e)),
        };

        let (url, checking_file) = match sample {
            // Имя файла проходит то же кодирование, что и зрительские ссылки:
            // иначе пробел или кириллица в имени сломают проверку там, где
            // сама раздача работает.
            Some(name) => (
                crate::domain::links::for_path(domain, None, name).origin,
                true,
            ),
            None => (
                format!("https://{domain}/{}/", crate::domain::links::VIDEOS_PREFIX),
                false,
            ),
        };

        // Просим один байт: этого хватает, чтобы убедиться в отдаче, и не тянет
        // с сервера гигабайты ради проверки.
        let request = if checking_file {
            client.get(&url).header("Range", "bytes=0-0")
        } else {
            client.get(&url)
        };

        match request.send().await {
            Ok(response) => {
                let code = response.status().as_u16();
                if !checking_file {
                    // Ответ веб-сервера — успех, но неполный: каталог может быть
                    // закрыт для перечисления, и это правильная настройка.
                    return said(
                        3,
                        StepStatus::Ok,
                        Detail::new(DetailCode::StepDomainOkNoFiles)
                            .with("domain", domain.to_owned())
                            .with("code", code),
                    );
                }
                if !response.status().is_success() {
                    return said(
                        3,
                        StepStatus::Failed,
                        Detail::new(DetailCode::StepDomainFileNotServed)
                            .with("url", url.clone())
                            .with("code", code),
                    );
                }
                match response.bytes().await {
                    Ok(body) if !body.is_empty() => said(
                        3,
                        StepStatus::Ok,
                        Detail::new(DetailCode::StepDomainOk).with("url", url.clone()),
                    ),
                    Ok(_) => said(
                        3,
                        StepStatus::Failed,
                        Detail::new(DetailCode::StepDomainEmptyBody)
                            .with("url", url.clone())
                            .with("code", code),
                    ),
                    Err(e) => said(3, StepStatus::Failed, system(&e)),
                }
            }
            Err(e) => {
                let detail = if e.is_timeout() {
                    Detail::new(DetailCode::StepDomainTimeout)
                        .with("domain", domain.to_owned())
                        .with("seconds", STEP_TIMEOUT.as_secs())
                } else if e.is_connect() {
                    Detail::new(DetailCode::StepDomainNoConnection)
                        .with("domain", domain.to_owned())
                } else {
                    system(&e)
                };
                said(3, StepStatus::Failed, detail)
            }
        }
    }
}
