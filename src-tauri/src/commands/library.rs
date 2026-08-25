//! T044–T049 — команды библиотеки.
//!
//! Договор: `contracts/ipc-commands.md`, раздел «Библиотека».
//!
//! Библиотека медиа-центрична: пользователь думает о произведении, а файлы — его
//! варианты. Поэтому наружу отдаётся не плоский перечень каталога, а список медиа
//! с вложенными файлами, и отдельной группой — то, что не удалось отнести ни к чему
//! (FR-015). Прятать нераспознанное нельзя: файл, которого не видно в приложении,
//! всё равно занимает место на диске и всё равно раздаётся по ссылке.

use super::error::{AppError, ErrorCode, Result};
use super::AppState;
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
    /// Сколько зрителей прямо сейчас получают эти файлы (FR-019a).
    pub active_viewers: usize,
}

pub mod api {
    use super::*;
    use crate::domain::links::Links;

    /// Библиотека сервера. Без `refresh` отдаёт кеш мгновенно и обновляет следом.
    pub async fn library_list(
        _state: &AppState,
        _server_id: &str,
        _refresh: bool,
    ) -> Result<LibraryView> {
        Err(not_implemented("library_list"))
    }

    /// Создать медиа. `slug` уникален в пределах сервера; пустой — составляется
    /// из названия.
    pub async fn media_create(
        _state: &AppState,
        _server_id: &str,
        _title: &str,
        _slug: Option<&str>,
    ) -> Result<String> {
        Err(not_implemented("media_create"))
    }

    /// Переименовать медиа. Смена `slug` переименовывает файлы и **ломает прежние
    /// ссылки** — интерфейс обязан предупредить об этом до вызова.
    pub async fn media_rename(
        _state: &AppState,
        _server_id: &str,
        _media_id: &str,
        _title: Option<&str>,
        _slug: Option<&str>,
    ) -> Result<()> {
        Err(not_implemented("media_rename"))
    }

    /// Удалить медиа вместе с файлами.
    ///
    /// Без `confirmed` выполняется **отказом**, в котором названы число файлов, объём
    /// и число зрителей: подтверждать вслепую нечего (FR-014, FR-019a).
    pub async fn media_delete(
        _state: &AppState,
        _server_id: &str,
        _media_id: &str,
        _confirmed: bool,
    ) -> Result<String> {
        Err(not_implemented("media_delete"))
    }

    /// Перенести файл в другое медиа.
    pub async fn file_move(
        _state: &AppState,
        _server_id: &str,
        _path: &str,
        _to_media_id: &str,
        _confirmed: bool,
    ) -> Result<()> {
        Err(not_implemented("file_move"))
    }

    /// Удалить один файл.
    pub async fn file_delete(
        _state: &AppState,
        _server_id: &str,
        _path: &str,
        _confirmed: bool,
    ) -> Result<()> {
        Err(not_implemented("file_delete"))
    }

    /// Зрительские ссылки на файл (FR-016).
    pub fn links_for(_state: &AppState, _server_id: &str, _path: &str) -> Result<Links> {
        Err(not_implemented("links_for"))
    }
}

fn not_implemented(what: &str) -> AppError {
    AppError::new(ErrorCode::Internal).with_cause(format!("{what}: ещё не реализовано"))
}
