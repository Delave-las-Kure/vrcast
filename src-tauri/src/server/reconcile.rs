//! T051 — свести опись с тем, что на самом деле лежит на сервере (FR-018).
//!
//! Расхождения неизбежны и нормальны: файлы заливают скриптами, удаляют руками,
//! переименовывают в файловом менеджере. Приложение не имеет права ни делать вид,
//! что этого не бывает, ни молча подгонять опись под факт.
//!
//! Два вида расхождения и два разных ответа:
//!
//! - **В описи есть, на сервере нет** — файл помечается пропавшим, но из медиа
//!   не исчезает. Убрать его молча значило бы скрыть от пользователя потерю.
//! - **На сервере есть, в описи нет** — файл попадает в группу «не распознано»
//!   (FR-015). Спрятать его нельзя: место он занимает и по ссылке отдаётся.
//!
//! Здесь только сведение — чистая функция от описи и перечня каталога. Ни сети,
//! ни файлов: сведение проверяется без сервера, потому что именно в нём легко
//! потерять файл, и такая потеря должна ловиться тестом, а не пользователем.

use super::listing::Entry;
use crate::domain::manifest::Manifest;
use std::collections::{HashMap, HashSet};

/// Результат сведения.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Reconciled {
    /// Для каждого медиа — его файлы с отметкой, существуют ли они.
    /// Порядок медиа и файлов взят из описи.
    pub media_files: Vec<MediaFiles>,
    /// Записи каталога, не числящиеся ни за одним медиа.
    pub unrecognized: Vec<Entry>,
}

/// Файлы одного медиа после сведения.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaFiles {
    pub media_id: String,
    /// Путь, размер и существует ли он на сервере.
    pub files: Vec<ResolvedFile>,
    /// Наборы качеств: путь и существует ли.
    pub ladders: Vec<ResolvedFile>,
}

/// Файл описи, сопоставленный с фактом.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedFile {
    pub path: String,
    pub size_bytes: u64,
    /// Ложь = в описи числится, а на сервере нет (FR-018).
    pub exists: bool,
}

/// Свести опись с перечнем каталога.
///
/// `entries` — верхний уровень каталога раздачи целиком, включая служебные записи:
/// отбор делается здесь, в одном месте.
pub fn reconcile(manifest: &Manifest, entries: &[Entry]) -> Reconciled {
    // Что реально есть, по имени верхнего уровня.
    let present: HashMap<&str, &Entry> = entries
        .iter()
        .filter(|e| !super::SERVICE_ENTRIES.contains(&e.name.as_str()))
        .map(|e| (e.name.as_str(), e))
        .collect();

    // Какие записи верхнего уровня заняты описью. Путь в описи может быть вложенным
    // (`backrooms/master.m3u8`) — занята им запись верхнего уровня `backrooms`.
    let mut claimed: HashSet<&str> = HashSet::new();
    let mut media_files = Vec::new();

    for media in &manifest.media {
        let resolve = |path: &String| -> ResolvedFile {
            let top = top_level(path);
            // Для вложенного пути размер записи верхнего уровня — это размер всего
            // набора качеств; приписывать его отдельному описанию нельзя.
            let entry = present.get(top);
            let nested = top != path.as_str();
            ResolvedFile {
                path: path.clone(),
                size_bytes: if nested {
                    0
                } else {
                    entry.map_or(0, |e| e.size_bytes)
                },
                exists: entry.is_some(),
            }
        };

        let files: Vec<ResolvedFile> = media.files.iter().map(&resolve).collect();
        let ladders: Vec<ResolvedFile> = media.ladders.iter().map(&resolve).collect();

        for path in media.all_paths() {
            claimed.insert(top_level(path));
        }

        media_files.push(MediaFiles {
            media_id: media.id.clone(),
            files,
            ladders,
        });
    }

    // Порядок нераспознанных берём из перечня каталога, а не из множества:
    // список видит человек, и он не должен меняться от запуска к запуску.
    let unrecognized: Vec<Entry> = entries
        .iter()
        .filter(|e| !super::SERVICE_ENTRIES.contains(&e.name.as_str()))
        .filter(|e| !claimed.contains(e.name.as_str()))
        .cloned()
        .collect();

    Reconciled {
        media_files,
        unrecognized,
    }
}

/// Запись верхнего уровня, которой принадлежит путь.
fn top_level(path: &str) -> &str {
    let trimmed = path.trim_matches('/');
    trimmed.split_once('/').map_or(trimmed, |(head, _)| head)
}
