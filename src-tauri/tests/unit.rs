//! Модульные тесты ядра.
//!
//! Файлы лежат в `tests/unit/`, а не рядом с этим файлом: путь указан явно, потому что
//! корень тестовой цели ищет модули в своём каталоге, а не в одноимённой папке.

#[path = "unit/db.rs"]
mod db;

#[path = "unit/domain_us1.rs"]
mod domain_us1;

#[path = "unit/moov.rs"]
mod moov;

#[path = "unit/engine.rs"]
mod engine;

#[path = "unit/process.rs"]
mod process;

#[path = "unit/registry.rs"]
mod registry;

#[path = "unit/redact.rs"]
mod redact;

#[path = "unit/ssh.rs"]
mod ssh;
