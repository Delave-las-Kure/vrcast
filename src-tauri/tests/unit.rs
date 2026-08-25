//! Модульные тесты ядра.
//!
//! Файлы лежат в `tests/unit/`, а не рядом с этим файлом: путь указан явно, потому что
//! корень тестовой цели ищет модули в своём каталоге, а не в одноимённой папке.

/// Общая оснастка. Проверки состояния процессов нужны и тестам запуска программ,
/// и тестам уборки: две копии одной проверки уже однажды дали разные ответы
/// на один вопрос.
#[path = "support/proc_check.rs"]
mod proc_check;

/// Ключ для тестов создаётся на месте и нужен и здесь, и интеграционным тестам.
#[path = "support/test_key.rs"]
mod test_key;

#[path = "unit/db.rs"]
mod db;

#[path = "unit/domain_us1.rs"]
mod domain_us1;

#[path = "unit/env_import.rs"]
mod env_import;

#[path = "unit/moov.rs"]
mod moov;

#[path = "unit/engine.rs"]
mod engine;

#[path = "unit/process.rs"]
mod process;

#[path = "unit/registry.rs"]
mod registry;

#[path = "unit/reconcile.rs"]
mod reconcile;

#[path = "unit/redact.rs"]
mod redact;

#[path = "unit/transfer.rs"]
mod transfer;

#[path = "unit/ssh.rs"]
mod ssh;

#[path = "unit/notify.rs"]
mod notify;

#[path = "unit/convert_plan.rs"]
mod convert_plan;
