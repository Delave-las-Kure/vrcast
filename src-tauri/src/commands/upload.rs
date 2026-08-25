//! T088, T094, T095 — команды заливки.
//!
//! Договор: `contracts/ipc-commands.md`, раздел «Заливка».
//!
//! Здесь живёт то, что нельзя оставить в слое передачи: правила повторов и
//! переподключения. Переподключение требует профиля и секрета, а тащить их в слой
//! передачи значило бы разложить работу с доступами по двум местам вместо одного.
//!
//! **Все проверки — до начала передачи** (FR-036, FR-037, FR-039). Узнать
//! о нехватке места в середине заливки на тридцать гигабайт значит потерять час
//! и оставить на сервере недокачанный хвост.

use super::error::{AppError, ErrorCode, Result};
use super::AppState;
use crate::domain::progress_estimate::ProgressEstimate;
use crate::domain::remote_name::{self, NameVerdict};
use crate::domain::transfer::ResumeToken;
use crate::server::free_space::{self, SpaceVerdict};
use crate::server::upload::{self, UploadError, UploadPlan};
use crate::server::{checksum, connect, disk, listing};
use crate::tasks::state::TaskKind;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;

/// Сколько раз пробуем переподключиться, прежде чем признать неудачу.
///
/// Обрыв на многочасовой передаче — обычное дело, а не поломка; сдаться после
/// первой же неудачи значило бы потребовать от человека сидеть рядом с кнопкой.
const MAX_ATTEMPTS: usize = 8;

/// С какой паузы начинаем повторять и до какой она растёт.
const FIRST_RETRY_DELAY: Duration = Duration::from_secs(2);
const MAX_RETRY_DELAY: Duration = Duration::from_secs(60);

/// Что интерфейс присылает для начала заливки.
#[derive(Debug, Clone, Deserialize)]
pub struct UploadRequest {
    pub server_id: String,
    /// Локальный путь к готовому файлу.
    pub local_path: String,
    /// Под каким именем файл станет виден зрителям.
    pub remote_name: String,
    /// К какому медиа отнести. Пусто — файл попадёт в «не распознано».
    pub media_id: Option<String>,
    /// Предел скорости в байтах в секунду. Пусто — не ограничивать.
    pub limit_bps: Option<u64>,
    /// Согласие на последствия, о которых предупредили до старта.
    #[serde(default)]
    pub confirmed: bool,
}

/// Что приложение обязано сказать **до** начала передачи.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Preflight {
    /// Не хватает места: сколько нужно и сколько есть.
    pub not_enough_space: Option<SpaceShortage>,
    /// Сколько соединений сервер обслуживает прямо сейчас: заливка вымоет из его
    /// памяти то, что смотрят, и просмотр подвиснет (FR-037).
    pub active_connections: usize,
    /// Файл с таким именем уже раздаётся (FR-039).
    pub name_exists: bool,
    /// При заданном CDN замена какое-то время будет отдаваться из кеша старой.
    pub cdn_cached: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpaceShortage {
    pub needed: u64,
    pub free: u64,
    pub short_by: u64,
}

impl Preflight {
    /// Есть ли о чём предупреждать.
    pub fn has_warnings(&self) -> bool {
        self.not_enough_space.is_some() || self.active_connections > 0 || self.name_exists
    }

    /// Нехватка места — не предупреждение, а запрет: подтверждением её не снять.
    pub fn is_blocking(&self) -> bool {
        self.not_enough_space.is_some()
    }
}

pub mod api {
    use super::*;
    use crate::store::profiles;

    /// Начать заливку.
    ///
    /// Возвращает номер задачи немедленно; сама передача идёт в движке задач
    /// (FR-080). Все проверки выполняются **до** постановки задачи: отказ должен
    /// прийти сразу, а не через час.
    pub async fn upload_start(state: &AppState, request: UploadRequest) -> Result<String> {
        let profile = profiles::get(&state.db, &request.server_id)?
            .ok_or_else(|| crate::commands::servers::no_such_server(&request.server_id))?;

        let local_path = PathBuf::from(&request.local_path);
        let meta = tokio::fs::metadata(&local_path).await.map_err(|e| {
            AppError::new(ErrorCode::InvalidInput)
                .with_message("Файл не найден или недоступен для чтения.")
                .with_cause(format!("{}: {e}", request.local_path))
        })?;
        if !meta.is_file() {
            return Err(AppError::new(ErrorCode::InvalidInput)
                .with_message("Указан не файл, а что-то другое."));
        }

        let clean_name = remote_name::sanitize(&request.remote_name);
        if clean_name.is_empty() {
            return Err(AppError::new(ErrorCode::InvalidInput)
                .with_message("Укажите имя, под которым файл станет виден зрителям."));
        }

        // Проверки до передачи — по живому соединению.
        let conn = connect(state.secrets.as_ref(), &profile).await?;
        let checks = preflight(&profile, &conn, &clean_name, meta.len()).await?;
        conn.close().await;

        if let Some(shortage) = checks.not_enough_space {
            return Err(space_error(shortage));
        }

        if checks.has_warnings() && !request.confirmed {
            return Err(warning_error(&checks, &clean_name));
        }

        // Две заливки под одним именем на один сервер писали бы в один временный
        // файл и затёрли бы работу друг друга — а узналось бы это только на сверке
        // контрольных сумм. Запрещаем прямо: имя временного файла нарочно зависит
        // только от конечного имени (см. `remote_name::staging_file`), и разводить
        // их номерами задач бессмысленно — при вводе в раздачу они всё равно
        // столкнутся.
        if let Some(busy) = running_upload_for(state, &profile.id, &clean_name)? {
            return Err(AppError::new(ErrorCode::NameExists)
                .with_message(format!(
                    "Файл «{clean_name}» уже заливается на этот сервер. \
                     Дождитесь конца или отмените ту задачу."
                ))
                .with_cause(busy));
        }

        // Всё проверено — ставим задачу.
        let db = state.db.clone();
        let secrets = state.secrets.clone();
        let plan_request = request.clone();
        let name_for_task = clean_name.clone();
        let total = meta.len();

        let task_id = state
            .tasks
            .submit(TaskKind::Upload, Some(profile.id.clone()), move |ctx| {
                let request = plan_request;
                let name = name_for_task;
                async move { run_upload(db, secrets, ctx, request, name, total).await }
            })
            .await?;

        Ok(task_id)
    }

    /// Продолжить приостановленную или прерванную заливку.
    pub fn upload_resume(state: &AppState, task_id: &str) -> Result<()> {
        Ok(state.tasks.resume(task_id)?)
    }

    /// Вернуть к жизни заливки, оставшиеся от прошлого запуска (FR-031).
    ///
    /// Вызывается один раз при старте приложения. Без этого заливка после закрытия
    /// и повторного запуска видна в списке приостановленной, но продолжить её нечем:
    /// рабочая часть живёт только в памяти и умирает вместе с приложением. Человеку
    /// это выглядело бы как «задача есть, а кнопка не работает».
    ///
    /// Задачи поднимаются **приостановленными** и ждут решения человека: самовольно
    /// продолжать многочасовую передачу при запуске нельзя — приложение могли закрыть
    /// именно ради её прекращения.
    ///
    /// Возвращает, сколько заливок поднято.
    pub fn restore_uploads(state: &AppState) -> Result<usize> {
        let mut restored = 0;

        for task in state.tasks.list()? {
            if task.kind != TaskKind::Upload || task.state.is_final() {
                continue;
            }
            let Some(token) = task.resume_token.as_deref().and_then(ResumeToken::parse) else {
                // Без позиции возобновления продолжать нечего: неизвестно ни куда
                // передавали, ни под каким именем. Такая задача остаётся в списке,
                // и её можно снять.
                tracing::debug!(task = %task.id, "заливка без позиции возобновления не поднята");
                continue;
            };
            let Some(server_id) = task.server_id.clone() else {
                continue;
            };
            let Ok(Some(_)) = profiles::get(&state.db, &server_id) else {
                tracing::debug!(task = %task.id, "сервер этой заливки удалён, поднимать некуда");
                continue;
            };

            // Путь к исходнику знает только позиция возобновления. Записи прежних
            // версий его не содержат — такую заливку продолжить нечем, но она
            // остаётся в списке, и её можно снять.
            let Some(local_path) = token.local_path.clone() else {
                tracing::warn!(task = %task.id, "в позиции возобновления нет пути к исходнику");
                continue;
            };

            let request = UploadRequest {
                server_id,
                local_path,
                remote_name: token.remote_name.clone(),
                media_id: token.media_id.clone(),
                limit_bps: token.limit_bps,
                // Человек согласился на последствия, когда начинал: спрашивать
                // второй раз о том же файле — значит не помнить его ответ.
                confirmed: true,
            };

            let db = state.db.clone();
            let secrets = state.secrets.clone();
            let name = token.remote_name.clone();
            let total = token.source_size;

            let result = state
                .tasks
                .resubmit_paused(&task.id, move |ctx| async move {
                    run_upload(db, secrets, ctx, request, name, total).await
                });

            match result {
                Ok(()) => restored += 1,
                Err(e) => tracing::warn!(task = %task.id, error = %e, "заливку не поднять"),
            }
        }

        if restored > 0 {
            tracing::info!(restored, "заливки прошлого запуска ждут продолжения");
        }
        Ok(restored)
    }

    /// Есть ли уже незавершённая заливка под этим именем на этот сервер.
    ///
    /// Оговорка про щель: позиция возобновления записывается уже внутри задачи,
    /// поэтому две заливки, начатые в одно и то же мгновение, эту проверку пройдут.
    /// Щель узкая и не последняя линия обороны: расхождение поймает сверка
    /// контрольных сумм, и в раздачу такой файл не попадёт. Закрывать её замком
    /// на всё время постановки задачи дороже, чем стоит случай.
    fn running_upload_for(state: &AppState, server_id: &str, name: &str) -> Result<Option<String>> {
        for task in state.tasks.list()? {
            if task.kind != TaskKind::Upload
                || task.state.is_final()
                || task.server_id.as_deref() != Some(server_id)
            {
                continue;
            }
            let same_target = task
                .resume_token
                .as_deref()
                .and_then(ResumeToken::parse)
                .is_some_and(|t| t.remote_name == name);
            if same_target {
                return Ok(Some(task.id));
            }
        }
        Ok(None)
    }

    /// Проверки, которые обязаны пройти до начала передачи.
    ///
    /// Отдельной командой наружу не выставлена намеренно: интерфейс узнаёт о
    /// последствиях тем же способом, что и при удалении — вызовом без подтверждения,
    /// на который приходит отказ с готовым текстом. Два разных способа спросить
    /// «точно?» разошлись бы формулировками.
    async fn preflight(
        profile: &crate::domain::server_profile::ServerProfile,
        conn: &crate::ssh::Connection,
        clean_name: &str,
        file_size: u64,
    ) -> Result<Preflight> {
        let usage = disk::usage(conn, &profile.video_dir).await?;

        // Сколько уже лежит во временном файле: при продолжении это место занято,
        // и требовать его заново значило бы отказать в докачке почти дошедшего файла.
        let staging = remote_name::staging_dir(&profile.video_dir).ok_or_else(|| {
            AppError::new(ErrorCode::InvalidInput).with_message(
                "Каталог раздачи указан в корне файловой системы — рядом с ним негде \
                 собирать файл, а собирать внутри нельзя: недокачанное стало бы видно зрителям.",
            )
        })?;
        let already =
            upload::uploaded_so_far(conn, &remote_name::staging_file(&staging, clean_name))
                .await
                .unwrap_or(0);

        let not_enough_space = match free_space::check(&usage, file_size, already) {
            SpaceVerdict::Fits => None,
            SpaceVerdict::NotEnough {
                needed,
                free,
                short_by,
            } => Some(SpaceShortage {
                needed,
                free,
                short_by,
            }),
        };

        let entries = listing::list(conn, &profile.video_dir).await?;
        let existing: Vec<String> = entries.into_iter().map(|e| e.name).collect();
        let verdict = remote_name::check_name(clean_name, &existing, profile.cdn_base.is_some());

        let (name_exists, cdn_cached) = match verdict {
            NameVerdict::Exists { cdn_cached } => (true, cdn_cached),
            NameVerdict::Reserved => {
                return Err(AppError::new(ErrorCode::InvalidInput)
                    .with_message("Это имя занято служебной записью раздачи — выберите другое."))
            }
            _ => (false, false),
        };

        Ok(Preflight {
            not_enough_space,
            active_connections: crate::server::active_use::serving_connections(conn).await,
            name_exists,
            cdn_cached,
        })
    }

    /// Отпечаток исходника: размер и время изменения.
    ///
    /// Одного размера мало — файл могли пересобрать в тот же объём, и тогда
    /// продолжение склеило бы две разные версии. Время изменения берётся как есть,
    /// без разбора: это метка для сравнения, а не дата для показа.
    async fn source_fingerprint(path: &str) -> std::result::Result<(u64, Option<String>), String> {
        let meta = tokio::fs::metadata(path)
            .await
            .map_err(|e| format!("исходный файл недоступен: {e}"))?;
        let modified = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs().to_string());
        Ok((meta.len(), modified))
    }

    /// Сама передача: попытки с переподключением, сверка, ввод в раздачу.
    async fn run_upload(
        db: std::sync::Arc<crate::store::db::Db>,
        secrets: std::sync::Arc<dyn crate::store::secrets::SecretStore>,
        ctx: crate::tasks::engine::TaskContext,
        request: UploadRequest,
        clean_name: String,
        total: u64,
    ) -> std::result::Result<(), String> {
        let profile = match crate::store::profiles::get(&db, &request.server_id) {
            Ok(Some(p)) => p,
            Ok(None) => return Err(String::from("Профиль сервера удалён — заливать некуда.")),
            Err(e) => return Err(e.to_string()),
        };

        let staging = remote_name::staging_dir(&profile.video_dir)
            .ok_or_else(|| String::from("Негде собирать файл рядом с каталогом раздачи."))?;

        // Тот ли это файл, с которого начинали.
        //
        // Проверка стоит **до** соединения с сервером: если исходник подменили,
        // продолжение допишет к началу одного файла хвост другого. Поймала бы это
        // и сверка контрольных сумм — но уже после того, как передача целиком
        // закончится, то есть через час работы впустую.
        let (size_now, modified_now) = source_fingerprint(&request.local_path).await?;
        let previous = ctx
            .resume_token()
            .ok()
            .flatten()
            .as_deref()
            .and_then(ResumeToken::parse);
        let source_changed = match &previous {
            // Продолжение прошлой передачи — сверяем с тем, что записали тогда.
            Some(prev) => !prev.matches_source(size_now, modified_now.as_deref()),
            // Первый заход — исходник мог измениться между проверками и стартом задачи.
            None => size_now != total,
        };
        if source_changed {
            return Err(String::from(
                "Исходный файл изменился с начала заливки. Продолжать нельзя: \
                 на сервере получилась бы смесь двух файлов. Начните заливку заново.",
            ));
        }

        let plan = UploadPlan {
            local_path: PathBuf::from(&request.local_path),
            remote_temp: remote_name::staging_file(&staging, &clean_name),
            remote_final: upload::final_path(&profile.video_dir, &clean_name),
            total_bytes: total,
            limit_bps: request.limit_bps,
        };

        // Позиция возобновления записывается сразу: если приложение убьют до первого
        // окна, следующий запуск обязан знать, куда смотреть.
        let token = ResumeToken {
            remote_temp: plan.remote_temp.clone(),
            remote_name: clean_name.clone(),
            local_path: Some(request.local_path.clone()),
            media_id: request.media_id.clone(),
            limit_bps: request.limit_bps,
            source_size: size_now,
            source_modified: modified_now,
        };
        let _ = ctx.save_resume_token(&token.to_json());

        let mut estimate = ProgressEstimate::default();
        let mut delay = FIRST_RETRY_DELAY;

        for attempt in 1..=MAX_ATTEMPTS {
            let conn = match connect(secrets.as_ref(), &profile).await {
                Ok(c) => c,
                Err(e) => {
                    if attempt == MAX_ATTEMPTS {
                        return Err(crate::store::redact::safe_display(&e));
                    }
                    wait_before_retry(&ctx, &mut delay).await?;
                    continue;
                }
            };

            if attempt == 1 {
                if let Err(e) = upload::ensure_staging(&conn, &staging, &profile.video_dir).await {
                    conn.close().await;
                    return Err(e.to_string());
                }
            }

            match upload::transfer_once(&conn, &ctx, &plan, &mut estimate).await {
                Ok(sent) => {
                    let outcome = finish(&conn, &ctx, &plan, sent, &clean_name, &request).await;
                    conn.close().await;
                    return outcome;
                }
                Err(UploadError::Cancelled) => {
                    upload::cleanup(&conn, &plan.remote_temp).await;
                    conn.close().await;
                    return Ok(());
                }
                Err(e) if e.is_retriable() && attempt < MAX_ATTEMPTS => {
                    tracing::warn!(attempt, error = %e, "передача оборвалась, пробуем снова");
                    // Оценка времени сбрасывается: накопленное до обрыва больше
                    // не описывает происходящее.
                    estimate.reset();
                    conn.close().await;
                    wait_before_retry(&ctx, &mut delay).await?;
                }
                Err(e) => {
                    conn.close().await;
                    return Err(e.to_string());
                }
            }
        }

        Err(String::from(
            "Передача обрывалась слишком много раз подряд. Проверьте связь и продолжите задачу.",
        ))
    }

    /// Сверить суммы и ввести файл в раздачу.
    async fn finish(
        conn: &crate::ssh::Connection,
        ctx: &crate::tasks::engine::TaskContext,
        plan: &UploadPlan,
        sent: u64,
        clean_name: &str,
        request: &UploadRequest,
    ) -> std::result::Result<(), String> {
        if sent != plan.total_bytes {
            return Err(format!(
                "На сервер попало {sent} байт из {}. Файл в раздачу не введён.",
                plan.total_bytes
            ));
        }

        ctx.report_important(0.98, "сверяем контрольные суммы");

        let ours = checksum::local(&plan.local_path)
            .await
            .map_err(|e| format!("не посчитать контрольную сумму исходника: {e}"))?;
        let theirs = checksum::remote(conn, &plan.remote_temp)
            .await
            .map_err(|e| crate::store::redact::safe_display(&e))?;

        if !checksum::matches(&ours, &theirs) {
            // Файл в раздачу не попадает, и мусор за собой убираем: испорченная
            // передача не должна оставлять следов (FR-032, FR-038).
            upload::cleanup(conn, &plan.remote_temp).await;
            return Err(String::from(
                "Переданный файл отличается от исходного. В раздачу он не введён, \
                 временные данные убраны — запустите заливку снова.",
            ));
        }

        upload::publish(conn, plan)
            .await
            .map_err(|e| e.to_string())?;

        ctx.report_important(1.0, "готово");
        let _ = request;
        let _ = clean_name;
        Ok(())
    }

    /// Подождать перед повтором, не пропустив отмену.
    async fn wait_before_retry(
        ctx: &crate::tasks::engine::TaskContext,
        delay: &mut Duration,
    ) -> std::result::Result<(), String> {
        let cancel = ctx.cancel_token();
        tokio::select! {
            _ = tokio::time::sleep(*delay) => {}
            _ = cancel.cancelled() => return Err(String::from("задача отменена")),
        }
        *delay = (*delay * 2).min(MAX_RETRY_DELAY);
        Ok(())
    }
}

/// Тонкие обёртки, которые оболочка выставляет интерфейсу.
pub mod ipc {
    use super::*;
    use tauri::State;

    #[tauri::command]
    pub async fn upload_start(
        state: State<'_, AppState>,
        request: UploadRequest,
    ) -> Result<String> {
        api::upload_start(&state, request).await
    }

    #[tauri::command]
    pub fn upload_resume(state: State<'_, AppState>, task_id: String) -> Result<()> {
        api::upload_resume(&state, &task_id)
    }
}

/// Отказ по нехватке места на сервере.
///
/// Подтверждением не снимается: место от согласия не появится. Отдельно от
/// [`warning_error`] именно поэтому — спутать запрет с предупреждением значило бы
/// предложить человеку кнопку «всё равно залить», после которой передача упрётся
/// в конец диска на середине.
pub fn space_error(shortage: SpaceShortage) -> AppError {
    AppError::new(ErrorCode::RemoteDiskFull)
        .with_message(format!(
            "На сервере не хватает {} — нужно {}, свободно {}.",
            human(shortage.short_by),
            human(shortage.needed),
            human(shortage.free)
        ))
        .with_cause(format!("short_by={}", shortage.short_by))
}

/// Отказ, который называет последствия и снимается подтверждением.
pub fn warning_error(checks: &Preflight, name: &str) -> AppError {
    let mut parts: Vec<String> = Vec::new();

    if checks.name_exists {
        parts.push(format!("Файл «{name}» уже раздаётся — он будет заменён."));
        if checks.cdn_cached {
            parts.push(String::from(
                "У CDN какое-то время останется прежняя копия, и зрители будут получать старое.",
            ));
        }
    }
    if checks.active_connections > 0 {
        parts.push(format!(
            "Прямо сейчас сервер отдаёт данные — открыто соединений: {}. \
             Заливка вымоет из его памяти то, что смотрят, и просмотр подвиснет.",
            checks.active_connections
        ));
    }

    let code = if checks.name_exists {
        ErrorCode::NameExists
    } else {
        ErrorCode::ViewersActive
    };
    AppError::new(code).with_message(parts.join(" "))
}

fn human(bytes: u64) -> String {
    const ЕДИНИЦЫ: [&str; 5] = ["Б", "КБ", "МБ", "ГБ", "ТБ"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < ЕДИНИЦЫ.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} Б")
    } else {
        format!("{value:.1} {}", ЕДИНИЦЫ[unit])
    }
}
