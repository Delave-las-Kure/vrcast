//! T044–T049 — команды библиотеки.
//!
//! Договор: `contracts/ipc-commands.md`, раздел «Библиотека».
//!
//! Библиотека медиа-центрична: пользователь думает о произведении, а файлы — его
//! варианты. Поэтому наружу отдаётся не плоский перечень каталога, а список медиа
//! с вложенными файлами, и отдельной группой — то, что не удалось отнести ни к чему
//! (FR-015). Прятать нераспознанное нельзя: файл, которого не видно в приложении,
//! всё равно занимает место на диске и всё равно раздаётся по ссылке.

use super::error::{AppError, DetailCode, ErrorCode, Result};
use super::AppState;
use crate::domain::wording::Detail;
use serde::{Deserialize, Serialize};

/// Файл раздачи в том виде, в каком его показывает интерфейс.
///
/// Ссылки здесь есть, хотя у `domain::media::MediaFile` их нет: там хранятся факты
/// о файле, а ссылка — вычисляемое представление, зависящее от профиля. Считать её
/// на границе — единственный способ не выдать устаревший адрес после смены домена.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileView {
    /// Путь относительно каталога видео.
    pub path: String,
    pub size_bytes: u64,
    pub duration_s: Option<f64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub bitrate_bps: Option<u64>,
    pub video_codec: Option<String>,
    pub audio_codec: Option<String>,
    /// `moov` в начале файла. Ложь = зритель будет ждать скачивания хвоста.
    pub faststart_ok: Option<bool>,
    /// Ложь = файл удалён или переименован мимо приложения (FR-018).
    pub exists_on_server: bool,
    pub origin_url: String,
    pub cdn_url: Option<String>,
}

/// Медиа со всеми его файлами.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaView {
    pub id: String,
    pub title: String,
    pub slug: String,
    pub files: Vec<FileView>,
    /// Описания наборов качеств.
    pub ladders: Vec<String>,
    /// Сколько всего занимают файлы медиа — то, что освободится при удалении.
    pub total_bytes: u64,
    pub created_at: String,
}

/// Место на диске сервера (FR-017).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiskUsage {
    pub total_bytes: u64,
    pub free_bytes: u64,
    /// Сколько из занятого приходится на каталог раздачи.
    pub used_by_videos_bytes: u64,
}

/// Библиотека целиком.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LibraryView {
    pub server_id: String,
    pub media: Vec<MediaView>,
    /// Файлы, которые не удалось отнести ни к одному медиа (FR-015).
    pub unrecognized: Vec<FileView>,
    /// `None`, когда сервер недоступен и место узнать неоткуда.
    pub disk: Option<DiskUsage>,
    /// Истина = показано последнее известное состояние, сервер сейчас недоступен.
    ///
    /// Пустой экран или бесконечная загрузка на недоступном сервере — худший из
    /// возможных ответов: пользователь не понимает, потерял он библиотеку или связь.
    pub stale: bool,
}

impl LibraryView {
    /// Сколько всего записей каталога учтено — файлов медиа, наборов качеств
    /// и нераспознанного вместе.
    ///
    /// Служит проверкой полноты: это число обязано совпадать с числом записей
    /// в каталоге раздачи на сервере, не считая служебных. Запись, не попавшая
    /// ни в медиа, ни в группу «не распознано», — потерянная запись: пользователь
    /// её не видит, а место она занимает и по ссылке отдаётся (FR-015).
    ///
    /// Набор качеств считается одной записью, а не сотней отрезков: пользователь
    /// думает о нём как о единице, и показывать ему каждый отрезок значило бы
    /// утопить библиотеку в шуме.
    pub fn accounted_entries(&self) -> usize {
        self.media
            .iter()
            .map(|m| m.files.len() + m.ladders.len())
            .sum::<usize>()
            + self.unrecognized.len()
    }
}

/// Что будет удалено — то, что пользователь обязан увидеть до подтверждения (FR-014).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeletionImpact {
    pub files: usize,
    pub bytes: u64,
    /// Сколько соединений веб-сервер обслуживает прямо сейчас.
    ///
    /// Именно соединений, а не зрителей этого файла: таблица соединений не говорит,
    /// что именно качают, и приписать их конкретному медиа пока нечем. В вехе A
    /// довольно факта наличия (FR-019a) — полноценный разбор появится в Фазе 4
    /// вместе со слежением за журналом раздачи. Называть это «зрителями файла»
    /// значило бы сказать пользователю то, чего мы не знаем.
    pub active_connections: usize,
}

/// Тонкие обёртки для оболочки. Логики здесь нет — только вызов `api`.
pub mod ipc {
    use super::*;
    use crate::domain::links::Links;
    use tauri::State;

    #[tauri::command]
    pub async fn library_list(
        state: State<'_, AppState>,
        server_id: String,
        refresh: Option<bool>,
    ) -> Result<LibraryView> {
        api::library_list(&state, &server_id, refresh.unwrap_or(false)).await
    }

    #[tauri::command]
    pub async fn media_create(
        state: State<'_, AppState>,
        server_id: String,
        title: String,
        slug: Option<String>,
    ) -> Result<String> {
        api::media_create(&state, &server_id, &title, slug.as_deref()).await
    }

    #[tauri::command]
    pub async fn media_rename(
        state: State<'_, AppState>,
        server_id: String,
        media_id: String,
        title: Option<String>,
        slug: Option<String>,
    ) -> Result<()> {
        api::media_rename(
            &state,
            &server_id,
            &media_id,
            title.as_deref(),
            slug.as_deref(),
        )
        .await
    }

    #[tauri::command]
    pub async fn media_delete(
        state: State<'_, AppState>,
        server_id: String,
        media_id: String,
        confirmed: Option<bool>,
    ) -> Result<String> {
        api::media_delete(&state, &server_id, &media_id, confirmed.unwrap_or(false)).await
    }

    #[tauri::command]
    pub async fn file_move(
        state: State<'_, AppState>,
        server_id: String,
        path: String,
        to_media_id: String,
        confirmed: Option<bool>,
    ) -> Result<()> {
        api::file_move(
            &state,
            &server_id,
            &path,
            &to_media_id,
            confirmed.unwrap_or(false),
        )
        .await
    }

    #[tauri::command]
    pub async fn file_delete(
        state: State<'_, AppState>,
        server_id: String,
        path: String,
        confirmed: Option<bool>,
    ) -> Result<()> {
        api::file_delete(&state, &server_id, &path, confirmed.unwrap_or(false)).await
    }

    #[tauri::command]
    pub fn links_for(state: State<'_, AppState>, server_id: String, path: String) -> Result<Links> {
        api::links_for(&state, &server_id, &path)
    }
}

/// Собрать сведения о файле для показа.
fn file_view(
    profile: &crate::domain::server_profile::ServerProfile,
    path: &str,
    size_bytes: u64,
    params: crate::server::probe_moov::FileParams,
    exists_on_server: bool,
) -> FileView {
    let links = crate::domain::links::for_path(&profile.domain, profile.cdn_base.as_deref(), path);
    FileView {
        path: path.to_owned(),
        size_bytes,
        duration_s: params.params.duration_s,
        width: params.params.width,
        height: params.params.height,
        bitrate_bps: params.params.bitrate_bps,
        video_codec: params.params.video_codec,
        audio_codec: params.params.audio_codec,
        faststart_ok: params.faststart_ok,
        exists_on_server,
        origin_url: links.origin,
        cdn_url: links.cdn,
    }
}

pub mod api {
    use super::*;
    use crate::domain::links::Links;
    use crate::domain::manifest::Manifest;
    use crate::domain::media::{self, Media};
    use crate::domain::server_profile::ServerProfile;
    use crate::server::{
        connect, disk, listing, manifest_io, probe_moov, reconcile, SERVICE_ENTRIES,
    };
    use crate::ssh::Connection;
    use crate::store::{library_cache, profiles};

    fn profile_of(state: &AppState, server_id: &str) -> Result<ServerProfile> {
        profiles::get(&state.db, server_id)?
            .ok_or_else(|| crate::commands::servers::no_such_server(server_id))
    }

    /// Библиотека сервера.
    ///
    /// Без `refresh` отдаётся кеш — мгновенно, — а обновление идёт следом и приходит
    /// событием. Ждать ответа сервера, чтобы показать список, который и так известен,
    /// незачем: по медленному каналу это секунды пустого экрана.
    pub async fn library_list(
        state: &AppState,
        server_id: &str,
        refresh: bool,
    ) -> Result<LibraryView> {
        let profile = profile_of(state, server_id)?;

        if !refresh {
            if let Some(cached) = library_cache::load(&state.db, server_id)? {
                // Обновление идёт своим ходом: пользователь уже видит список,
                // а расхождение с сервером придёт событием и поправит показ.
                spawn_background_refresh(state.clone(), profile.clone());
                return Ok(cached);
            }
        }

        match build_from_server(state, &profile).await {
            Ok(view) => {
                library_cache::save(&state.db, server_id, &view)?;
                Ok(view)
            }
            Err(e) => {
                // Сервер недоступен. Показать последнее известное с пометкой лучше,
                // чем пустой экран: пустой неотличим от «библиотека пропала».
                match library_cache::load(&state.db, server_id)? {
                    Some(mut cached) => {
                        tracing::warn!(server = server_id, error = %e, "библиотека взята из кеша");
                        cached.stale = true;
                        Ok(cached)
                    }
                    None => Err(e),
                }
            }
        }
    }

    /// Обновить кеш в стороне от ответа и сообщить об изменении.
    fn spawn_background_refresh(state: AppState, profile: ServerProfile) {
        let server_id = profile.id.clone();
        tokio::spawn(async move {
            match build_from_server(&state, &profile).await {
                Ok(view) => {
                    if library_cache::save(&state.db, &server_id, &view).is_ok() {
                        state.notify_library_changed(&server_id);
                    }
                }
                Err(e) => {
                    tracing::debug!(server = %server_id, error = %e, "фоновое обновление библиотеки не удалось")
                }
            }
        });
    }

    /// Прочитать библиотеку с сервера целиком.
    async fn build_from_server(state: &AppState, profile: &ServerProfile) -> Result<LibraryView> {
        let conn = connect(state.secrets.as_ref(), profile).await?;
        let dir = &profile.video_dir;

        let manifest = manifest_io::read(&conn, dir).await?;
        let entries = listing::list(&conn, dir).await?;
        let matched = reconcile::reconcile(&manifest, &entries);

        // Место на диске — не повод отказать в библиотеке: если его не узнать,
        // список всё равно полезен.
        let disk_usage = match disk::usage(&conn, dir).await {
            Ok(u) => Some(u),
            Err(e) => {
                tracing::warn!(error = %e, "место на диске сервера не прочитано");
                None
            }
        };

        let mut media_views = Vec::with_capacity(manifest.media.len());
        for (media, files) in manifest.media.iter().zip(matched.media_files.iter()) {
            let mut views = Vec::with_capacity(files.files.len());
            let mut total = 0u64;
            for f in &files.files {
                total += f.size_bytes;
                let params = probed(state, &conn, profile, &f.path, f.size_bytes, f.exists).await;
                views.push(file_view(profile, &f.path, f.size_bytes, params, f.exists));
            }
            // Набор качеств в объём медиа входит: удаление освободит и его.
            total += files.ladders.iter().map(|l| l.size_bytes).sum::<u64>();

            media_views.push(MediaView {
                id: media.id.clone(),
                title: media.title.clone(),
                slug: media.slug.clone(),
                files: views,
                ladders: files.ladders.iter().map(|l| l.path.clone()).collect(),
                total_bytes: total,
                created_at: media.created_at.clone(),
            });
        }

        let mut unrecognized = Vec::with_capacity(matched.unrecognized.len());
        for entry in &matched.unrecognized {
            // Внутрь каталога не заглядываем: заголовка у каталога нет, а перебирать
            // его содержимое ради неизвестно чего — лишние обороты по сети.
            let params = if entry.is_dir {
                probe_moov::FileParams::default()
            } else {
                probed(state, &conn, profile, &entry.name, entry.size_bytes, true).await
            };
            unrecognized.push(file_view(
                profile,
                &entry.name,
                entry.size_bytes,
                params,
                true,
            ));
        }

        conn.close().await;

        Ok(LibraryView {
            server_id: profile.id.clone(),
            media: media_views,
            unrecognized,
            disk: disk_usage,
            stale: false,
        })
    }

    /// Параметры файла из заголовка. Неудача разбора — не повод потерять сам файл.
    async fn probed(
        state: &AppState,
        conn: &Connection,
        profile: &ServerProfile,
        path: &str,
        size_bytes: u64,
        exists: bool,
    ) -> probe_moov::FileParams {
        if !exists || size_bytes == 0 || path.contains('/') {
            return probe_moov::FileParams::default();
        }
        probe_moov::params_for(
            conn,
            &state.db,
            &profile.id,
            &profile.video_dir,
            path,
            size_bytes,
        )
        .await
        .unwrap_or_else(|e| {
            tracing::debug!(file = path, error = %e, "заголовок файла не прочитан");
            probe_moov::FileParams::default()
        })
    }

    /// Создать медиа. `slug` уникален в пределах сервера; пустой — составляется
    /// из названия.
    pub async fn media_create(
        state: &AppState,
        server_id: &str,
        title: &str,
        slug: Option<&str>,
    ) -> Result<String> {
        let profile = profile_of(state, server_id)?;

        let title = title.trim();
        if title.is_empty() {
            return Err(AppError::new(ErrorCode::InvalidInput).detail(DetailCode::MediaTitleEmpty));
        }

        let slug = match slug.map(str::trim).filter(|s| !s.is_empty()) {
            Some(s) => s.to_owned(),
            None => media::slugify(title).ok_or_else(|| {
                AppError::new(ErrorCode::InvalidInput).detail(DetailCode::SlugUnmakeable)
            })?,
        };
        media::validate_slug(&slug)
            .map_err(|e| AppError::new(ErrorCode::InvalidInput).with_detail(e.detail()))?;

        let conn = connect(state.secrets.as_ref(), &profile).await?;
        let manifest = manifest_io::read(&conn, &profile.video_dir).await?;

        if !manifest.slug_available(&slug, None) {
            conn.close().await;
            return Err(AppError::new(ErrorCode::SlugTaken).with_cause(&slug));
        }

        let id = format!("m_{}", uuid::Uuid::new_v4().simple());
        let mut next = manifest.prepared_for_write();
        next.media.push(Media::new(
            &id,
            title,
            &slug,
            crate::store::db::now_rfc3339(),
        ));

        manifest_io::write(&conn, &profile.video_dir, &next, manifest.generation).await?;
        conn.close().await;

        invalidate(state, server_id);
        Ok(id)
    }

    /// Переименовать медиа.
    ///
    /// Смена короткого имени переименовывает файлы и **делает прежние ссылки
    /// нерабочими**: интерфейс обязан предупредить об этом до вызова.
    pub async fn media_rename(
        state: &AppState,
        server_id: &str,
        media_id: &str,
        title: Option<&str>,
        slug: Option<&str>,
    ) -> Result<()> {
        let profile = profile_of(state, server_id)?;

        let new_title = title.map(str::trim).filter(|t| !t.is_empty());
        let new_slug = slug.map(str::trim).filter(|s| !s.is_empty());
        if new_title.is_none() && new_slug.is_none() {
            return Err(
                AppError::new(ErrorCode::InvalidInput).detail(DetailCode::MediaNothingToChange)
            );
        }
        if let Some(s) = new_slug {
            media::validate_slug(s)
                .map_err(|e| AppError::new(ErrorCode::InvalidInput).with_detail(e.detail()))?;
        }

        let conn = connect(state.secrets.as_ref(), &profile).await?;
        let manifest = manifest_io::read(&conn, &profile.video_dir).await?;

        let Some(index) = manifest.media.iter().position(|m| m.id == media_id) else {
            conn.close().await;
            return Err(no_such_media(media_id));
        };
        if let Some(s) = new_slug {
            if !manifest.slug_available(s, Some(media_id)) {
                conn.close().await;
                return Err(AppError::new(ErrorCode::SlugTaken).with_cause(s));
            }
        }

        let mut next = manifest.prepared_for_write();
        let media = &mut next.media[index];
        if let Some(t) = new_title {
            media.title = t.to_owned();
        }

        if let Some(s) = new_slug {
            let old = media.slug.clone();
            if s != old {
                rename_entries(&conn, &profile.video_dir, media, &old, s).await?;
                media.slug = s.to_owned();
            }
        }

        manifest_io::write(&conn, &profile.video_dir, &next, manifest.generation).await?;
        conn.close().await;

        invalidate(state, server_id);
        Ok(())
    }

    /// Переименовать записи каталога вслед за коротким именем.
    ///
    /// Переименование идёт **до** записи описи: если оно не удастся, опись останется
    /// прежней и будет соответствовать тому, что на сервере. Обратный порядок дал бы
    /// опись, ссылающуюся на несуществующие файлы.
    async fn rename_entries(
        conn: &Connection,
        video_dir: &str,
        media: &mut Media,
        old_slug: &str,
        new_slug: &str,
    ) -> Result<()> {
        use crate::server::{join_remote, shell_quote};

        // Записи верхнего уровня, которые надо переименовать: имя целиком равно
        // старому короткому имени либо начинается с него.
        let mut renames: Vec<(String, String)> = Vec::new();
        let mut rename_top = |path: &str| -> String {
            let (top, rest) = match path.split_once('/') {
                Some((t, r)) => (t, Some(r)),
                None => (path, None),
            };
            let new_top = if top == old_slug {
                new_slug.to_owned()
            } else if let Some(tail) = top.strip_prefix(old_slug) {
                format!("{new_slug}{tail}")
            } else {
                // Файл не следует соглашению об именах — трогать его нельзя:
                // пользователь мог отнести к медиа что-то со своим именем.
                return path.to_owned();
            };
            if top != new_top && !renames.iter().any(|(o, _)| o == top) {
                renames.push((top.to_owned(), new_top.clone()));
            }
            match rest {
                Some(r) => format!("{new_top}/{r}"),
                None => new_top,
            }
        };

        let new_files: Vec<String> = media.files.iter().map(|p| rename_top(p)).collect();
        let new_ladders: Vec<String> = media.ladders.iter().map(|p| rename_top(p)).collect();

        for (old, new) in &renames {
            let out = conn
                .exec(&format!(
                    "mv -n -- {} {}",
                    shell_quote(&join_remote(video_dir, old)),
                    shell_quote(&join_remote(video_dir, new))
                ))
                .await?;
            if !out.ok() {
                return Err(AppError::new(ErrorCode::Internal)
                    .with_detail(
                        Detail::new(DetailCode::RenameFailed)
                            .with("old", old.to_string())
                            .with("new", new.to_string()),
                    )
                    .with_cause(out.stderr.trim()));
            }
        }

        media.files = new_files;
        media.ladders = new_ladders;
        Ok(())
    }

    /// Удалить медиа вместе с файлами.
    ///
    /// Без `confirmed` выполняется **отказом**, в котором названы число файлов и
    /// объём: подтверждать вслепую нечего (FR-014).
    ///
    /// Удаление не задача, хотя договор называет `task_id`: снятие файлов — это
    /// операция по числу файлов, а не по их объёму, и занимает доли секунды даже
    /// на десятках гигабайт. Возвращается номер удалённого медиа.
    pub async fn media_delete(
        state: &AppState,
        server_id: &str,
        media_id: &str,
        confirmed: bool,
    ) -> Result<String> {
        let profile = profile_of(state, server_id)?;
        let conn = connect(state.secrets.as_ref(), &profile).await?;
        let manifest = manifest_io::read(&conn, &profile.video_dir).await?;

        let Some(index) = manifest.media.iter().position(|m| m.id == media_id) else {
            conn.close().await;
            return Err(no_such_media(media_id));
        };

        if !confirmed {
            let impact = impact_of(&conn, &profile, &manifest, index).await;
            conn.close().await;
            return Err(confirmation_needed(&manifest.media[index].title, &impact));
        }

        let media = manifest.media[index].clone();
        remove_entries(&conn, &profile.video_dir, media.all_paths()).await?;

        let mut next = manifest.prepared_for_write();
        next.media.remove(index);
        manifest_io::write(&conn, &profile.video_dir, &next, manifest.generation).await?;
        conn.close().await;

        invalidate(state, server_id);
        tracing::info!(media = media_id, "медиа удалено вместе с файлами");
        Ok(media_id.to_owned())
    }

    /// Перенести файл в другое медиа.
    ///
    /// Файл остаётся на месте — меняется только то, за каким медиа он числится.
    /// Переименовывать его вслед за новым коротким именем нельзя: это оборвало бы
    /// работающие ссылки, о чём пользователь не просил.
    pub async fn file_move(
        state: &AppState,
        server_id: &str,
        path: &str,
        to_media_id: &str,
        _confirmed: bool,
    ) -> Result<()> {
        let profile = profile_of(state, server_id)?;
        let conn = connect(state.secrets.as_ref(), &profile).await?;
        let manifest = manifest_io::read(&conn, &profile.video_dir).await?;

        if manifest.find_by_id(to_media_id).is_none() {
            conn.close().await;
            return Err(no_such_media(to_media_id));
        }

        let entries = listing::list(&conn, &profile.video_dir).await?;
        let top = path.split('/').next().unwrap_or(path);
        if !entries.iter().any(|e| e.name == top) {
            conn.close().await;
            return Err(AppError::new(ErrorCode::FileMissingOnServer).with_cause(path));
        }

        let mut next = manifest.prepared_for_write();
        for m in &mut next.media {
            m.files.retain(|p| p != path);
            m.ladders.retain(|p| p != path);
        }
        if let Some(target) = next.media.iter_mut().find(|m| m.id == to_media_id) {
            if path.ends_with(".m3u8") {
                target.ladders.push(path.to_owned());
            } else {
                target.files.push(path.to_owned());
            }
        }

        manifest_io::write(&conn, &profile.video_dir, &next, manifest.generation).await?;
        conn.close().await;

        invalidate(state, server_id);
        Ok(())
    }

    /// Удалить один файл.
    ///
    /// Без `confirmed` — отказ с объёмом, который освободится (FR-014).
    pub async fn file_delete(
        state: &AppState,
        server_id: &str,
        path: &str,
        confirmed: bool,
    ) -> Result<()> {
        let profile = profile_of(state, server_id)?;
        let conn = connect(state.secrets.as_ref(), &profile).await?;

        let entries = listing::list(&conn, &profile.video_dir).await?;
        let top = path.split('/').next().unwrap_or(path);
        let Some(entry) = entries.iter().find(|e| e.name == top) else {
            conn.close().await;
            return Err(AppError::new(ErrorCode::FileMissingOnServer).with_cause(path));
        };
        if SERVICE_ENTRIES.contains(&top) {
            conn.close().await;
            return Err(
                AppError::new(ErrorCode::InvalidInput).detail(DetailCode::MediaIsServiceEntry)
            );
        }

        if !confirmed {
            let impact = DeletionImpact {
                files: 1,
                bytes: entry.size_bytes,
                active_connections: active_connections(&conn).await,
            };
            conn.close().await;
            return Err(confirmation_needed(path, &impact));
        }

        remove_entries(&conn, &profile.video_dir, std::iter::once(&path.to_owned())).await?;

        // Из описи убираем тем же действием: файла нет, и ссылка на него в описи
        // превратилась бы в вечно пропавший файл.
        let manifest = manifest_io::read(&conn, &profile.video_dir).await?;
        if manifest.all_claimed_paths().contains(&path) {
            let mut next = manifest.prepared_for_write();
            for m in &mut next.media {
                m.files.retain(|p| p != path);
                m.ladders.retain(|p| p != path);
            }
            manifest_io::write(&conn, &profile.video_dir, &next, manifest.generation).await?;
        }
        conn.close().await;

        invalidate(state, server_id);
        Ok(())
    }

    /// Зрительские ссылки на файл (FR-016).
    pub fn links_for(state: &AppState, server_id: &str, path: &str) -> Result<Links> {
        let profile = profile_of(state, server_id)?;
        Ok(crate::domain::links::for_path(
            &profile.domain,
            profile.cdn_base.as_deref(),
            path,
        ))
    }

    // ---------- вспомогательное ----------

    fn no_such_media(id: &str) -> AppError {
        AppError::new(ErrorCode::InvalidInput)
            .detail(DetailCode::MediaNotFound)
            .with_cause(id)
    }

    /// Отказ, который называет последствия. Без чисел подтверждать было бы нечего.
    /// A refusal that names the consequences. Without the numbers there would be
    /// nothing to confirm.
    fn confirmation_needed(what: &str, impact: &DeletionImpact) -> AppError {
        let mut error = AppError::new(ErrorCode::ConfirmationRequired).with_detail(
            Detail::new(DetailCode::ConfirmDelete)
                .with("what", what.to_string())
                .with("files", impact.files)
                .with("bytes", impact.bytes),
        );
        // A second thing to say, not a longer first one: whether anyone is watching
        // right now is a separate fact, and it is worded the same wherever it comes up.
        if impact.active_connections > 0 {
            error = error.with_detail(
                Detail::new(DetailCode::ViewersActiveDelete)
                    .with("connections", impact.active_connections),
            );
        }
        error.with_cause(format!(
            "files={}, bytes={}, connections={}",
            impact.files, impact.bytes, impact.active_connections
        ))
    }

    async fn impact_of(
        conn: &Connection,
        profile: &ServerProfile,
        manifest: &Manifest,
        index: usize,
    ) -> DeletionImpact {
        let media = &manifest.media[index];
        let entries = listing::list(conn, &profile.video_dir)
            .await
            .unwrap_or_default();
        let matched = reconcile::reconcile(manifest, &entries);

        let files = matched.media_files.get(index);
        let bytes = files.map_or(0, |f| {
            f.files.iter().map(|x| x.size_bytes).sum::<u64>()
                + f.ladders.iter().map(|x| x.size_bytes).sum::<u64>()
        });

        DeletionImpact {
            files: media.files.len() + media.ladders.len(),
            bytes,
            active_connections: active_connections(conn).await,
        }
    }

    /// Сколько соединений веб-сервер обслуживает прямо сейчас (FR-019a).
    ///
    /// Живёт в `server::active_use`: то же самое нужно и перед заливкой (FR-037),
    /// и две копии одного подсчёта разошлись бы при первой же правке.
    async fn active_connections(conn: &Connection) -> usize {
        crate::server::active_use::serving_connections(conn).await
    }

    /// Удалить записи каталога — и файлы, и каталоги наборов качеств.
    async fn remove_entries<'a>(
        conn: &Connection,
        video_dir: &str,
        paths: impl Iterator<Item = &'a String>,
    ) -> Result<()> {
        use crate::server::{join_remote, shell_quote};

        // Удаляются записи верхнего уровня: набор качеств — это каталог целиком,
        // и снимать его по отрезку значило бы оставить половину.
        let mut tops: Vec<String> = Vec::new();
        for path in paths {
            let top = path.split('/').next().unwrap_or(path).to_owned();
            if SERVICE_ENTRIES.contains(&top.as_str()) {
                continue;
            }
            if !tops.contains(&top) {
                tops.push(top);
            }
        }
        if tops.is_empty() {
            return Ok(());
        }

        let args = tops
            .iter()
            .map(|t| shell_quote(&join_remote(video_dir, t)))
            .collect::<Vec<_>>()
            .join(" ");
        let out = conn.exec(&format!("rm -rf -- {args}")).await?;
        if !out.ok() {
            return Err(AppError::new(ErrorCode::Internal)
                .detail(DetailCode::DeleteFilesFailed)
                .with_cause(out.stderr.trim()));
        }
        Ok(())
    }

    /// Забыть кеш: после изменения он заведомо не соответствует серверу.
    fn invalidate(state: &AppState, server_id: &str) {
        if let Err(e) = library_cache::forget(&state.db, server_id) {
            tracing::warn!(server = server_id, error = %e, "кеш библиотеки не сброшен");
        }
        state.notify_library_changed(server_id);
    }
}
