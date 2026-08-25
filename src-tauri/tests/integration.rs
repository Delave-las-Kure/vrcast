//! Интеграционные тесты против одноразового сервера в контейнере (T026).
//!
//! Выключены по умолчанию: требуют Docker и идут заметно дольше модульных.
//! Запуск:
//!
//! ```text
//! cargo test --features integration --test integration -- --test-threads=1
//! ```
//!
//! Однопоточность обязательна: каждый тест поднимает свой контейнер, а параллельный
//! запуск дюжины контейнеров на обычной машине только замедляет дело.
//!
//! Боевой сервер здесь не используется НИКОГДА — конституция, раздел «Порядок работы».

#![cfg(feature = "integration")]

/// Общая оснастка: ключ для тестов создаётся на месте и нужен и здесь,
/// и модульным тестам.
#[path = "support/test_key.rs"]
mod test_key;

/// Проверки состояния процессов — общие с модульными тестами.
#[path = "support/proc_check.rs"]
mod proc_check;

#[path = "integration/convert_kill.rs"]
mod convert_kill;

#[path = "integration/fixture.rs"]
mod fixture;

#[path = "integration/library_completeness.rs"]
mod library_completeness;

#[path = "integration/library_ops.rs"]
mod library_ops;

#[path = "integration/live_readonly.rs"]
mod live_readonly;

#[path = "integration/manifest_conflict.rs"]
mod manifest_conflict;

#[path = "integration/process_tree.rs"]
mod process_tree;

#[path = "integration/upload_live.rs"]
mod upload_live;

#[path = "integration/ssh_live.rs"]
mod ssh_live;
