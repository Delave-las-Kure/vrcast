//! T008 — локальное хранилище: подключение, схема, миграции.
//!
//! Здесь лежит то, что переживает перезапуск приложения: профили серверов **без секретов**
//! (сами секреты — в хранилище ОС, см. `super::secrets`), журнал задач с позицией возобновления
//! и кеш библиотеки. Требования: FR-081 (задачи переживают перезапуск), FR-085 (повтор безопасен),
//! конституция, принцип V.

use rusqlite::Connection;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

/// Версия схемы, которую понимает эта сборка приложения.
pub const SCHEMA_VERSION: u32 = 1;

/// Миграции применяются по порядку; номер = значение `user_version` после применения.
/// Менять уже выпущенную миграцию нельзя — только добавлять следующую.
const MIGRATIONS: &[(u32, &str)] = &[(1, include_str!("migrations/0001_initial.sql"))];

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("не удалось открыть локальную базу: {0}")]
    Open(#[source] rusqlite::Error),

    #[error("не удалось применить миграцию {version}: {source}")]
    Migration {
        version: u32,
        #[source]
        source: rusqlite::Error,
    },

    /// Тот же принцип, что и с версией серверной части (FR-130): встретив состояние новее,
    /// чем понимаем, мы отказываемся работать, а не портим его молча.
    #[error("локальная база создана более новой версией приложения (версия схемы {found}, эта сборка знает до {known}). Обновите приложение")]
    TooNew { found: u32, known: u32 },

    #[error("не удалось определить каталог данных приложения")]
    NoDataDir,

    #[error(transparent)]
    Sql(#[from] rusqlite::Error),
}

pub type Result<T> = std::result::Result<T, DbError>;

/// Локальная база.
///
/// `Connection` из rusqlite не разделяется между потоками, поэтому доступ идёт через мьютекс.
/// Критические участки короткие: длительная работа (передача, кодирование) базу не держит,
/// она пишет только отметки о сдвигах состояния.
pub struct Db {
    conn: Mutex<Connection>,
}

impl Db {
    /// Открыть базу по пути, создав каталог при необходимости, и применить миграции.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| {
                DbError::Open(rusqlite::Error::InvalidPath(
                    format!("{}: {e}", dir.display()).into(),
                ))
            })?;
        }
        let conn = Connection::open(path).map_err(DbError::Open)?;
        Self::from_conn(conn)
    }

    /// База в памяти — для тестов. Никаких следов на диске.
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory().map_err(DbError::Open)?;
        Self::from_conn(conn)
    }

    fn from_conn(conn: Connection) -> Result<Self> {
        // Ссылочная целостность в SQLite выключена по умолчанию — включаем явно,
        // иначе удаление профиля оставит осиротевшие задачи.
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        // Журнал с упреждающей записью: чтение не блокируется записью. На базе в памяти
        // не поддерживается и молча остаётся прежним — это нормально.
        let _: String = conn.query_row("PRAGMA journal_mode = WAL", [], |r| r.get(0))?;
        conn.busy_timeout(Duration::from_secs(5))?;

        let db = Self {
            conn: Mutex::new(conn),
        };
        db.migrate()?;
        Ok(db)
    }

    /// Путь к базе по умолчанию: каталог данных приложения текущего пользователя.
    pub fn default_path() -> Result<PathBuf> {
        let dirs = directories::ProjectDirs::from("ru", "VRCast", "VRCast Studio")
            .ok_or(DbError::NoDataDir)?;
        Ok(dirs.data_dir().join("vrcast-studio.sqlite"))
    }

    /// Применить недостающие миграции. Безопасно при повторе: уже применённые пропускаются.
    fn migrate(&self) -> Result<()> {
        let mut guard = self.conn.lock().expect("мьютекс базы отравлен");
        let conn = &mut *guard;

        let current: u32 =
            conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))? as u32;

        if current > SCHEMA_VERSION {
            return Err(DbError::TooNew {
                found: current,
                known: SCHEMA_VERSION,
            });
        }

        for (version, sql) in MIGRATIONS {
            if *version <= current {
                continue;
            }
            // Миграция и отметка о ней — в одной транзакции: иначе прерывание между
            // ними оставит базу в состоянии, которое не соответствует записанной версии.
            let tx = conn.transaction()?;
            tx.execute_batch(sql).map_err(|source| DbError::Migration {
                version: *version,
                source,
            })?;
            tx.pragma_update(None, "user_version", *version as i64)?;
            tx.commit()?;
            tracing::info!(version = *version, "применена миграция базы");
        }
        Ok(())
    }

    /// Текущая версия схемы.
    pub fn schema_version(&self) -> Result<u32> {
        let conn = self.conn.lock().expect("мьютекс базы отравлен");
        Ok(conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))? as u32)
    }

    /// Выполнить работу с подключением. Держите замыкание коротким.
    pub fn with_conn<T>(&self, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        let conn = self.conn.lock().expect("мьютекс базы отравлен");
        f(&conn)
    }

    /// То же, но с изменяемым доступом — нужно для транзакций.
    pub fn with_conn_mut<T>(&self, f: impl FnOnce(&mut Connection) -> Result<T>) -> Result<T> {
        let mut conn = self.conn.lock().expect("мьютекс базы отравлен");
        f(&mut conn)
    }
}

/// Метка времени в виде, пригодном для хранения и сравнения строками.
pub fn now_rfc3339() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| String::from("1970-01-01T00:00:00Z"))
}
