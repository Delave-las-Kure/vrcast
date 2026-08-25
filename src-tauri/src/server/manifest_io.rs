//! T045 — чтение и запись описи библиотеки на сервере.
//!
//! Порядок записи обязателен (R-10, `contracts/server-contract.md`):
//! прочитать с поколением → изменить → записать во временный файл рядом → атомарно
//! заменить. И между чтением и заменой поколение на сервере обязано быть проверено
//! ещё раз: иначе второй экземпляр приложения молча сотрёт работу первого.
//!
//! Почему «рядом», а не поверх: запись поверх — это окно, в котором на сервере лежит
//! наполовину записанный файл. Если связь оборвётся именно там, библиотека окажется
//! потеряна не наполовину, а целиком — разобрать обрезанный JSON нечем.

use crate::domain::manifest::Manifest;
use crate::ssh::Connection;

/// Имя файла описи внутри каталога раздачи.
pub const MANIFEST_NAME: &str = "library.json";

#[derive(Debug, thiserror::Error)]
pub enum ManifestIoError {
    /// Опись изменена другим экземпляром приложения между чтением и записью.
    /// Запись **не выполнена**: на сервере осталось чужое изменение.
    #[error("опись изменена другим приложением: прочитано поколение {base}, на сервере {current}")]
    Conflict { base: u64, current: u64 },

    #[error("опись не разобрать: {0}")]
    Malformed(String),

    #[error(transparent)]
    Ssh(#[from] crate::ssh::SshError),
}

pub type Result<T> = std::result::Result<T, ManifestIoError>;

/// Прочитать опись. Отсутствие файла — пустая библиотека, а не ошибка.
pub async fn read(_conn: &Connection, _video_dir: &str) -> Result<Manifest> {
    Err(not_implemented())
}

/// Записать опись, если на сервере всё ещё `base_generation`.
///
/// `manifest.generation` обязан быть на единицу больше `base_generation` —
/// это заявка «записываю поверх того, что прочитал» (см. `Manifest::prepared_for_write`).
pub async fn write(
    _conn: &Connection,
    _video_dir: &str,
    _manifest: &Manifest,
    _base_generation: u64,
) -> Result<()> {
    Err(not_implemented())
}

fn not_implemented() -> ManifestIoError {
    ManifestIoError::Malformed(String::from("ещё не реализовано"))
}
