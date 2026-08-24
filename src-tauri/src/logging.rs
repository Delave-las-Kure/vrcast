//! Настройка вывода в журнал.
//!
//! Единственная точка, где журнал соединяется с потоком вывода, — и потому единственное
//! место, где нужно поставить вырезание секретов, чтобы оно действовало на всё
//! (конституция, принцип IV).

use crate::store::redact::RedactingMakeWriter;
use tracing_subscriber::EnvFilter;

/// Включить журнал. Вызывается один раз при запуске приложения.
///
/// Повторный вызов ничего не ломает: установка глобального получателя молча не удастся,
/// и уже действующая настройка сохранится.
pub fn init() {
    let filter = EnvFilter::try_from_env("VRCAST_LOG").unwrap_or_else(|_| EnvFilter::new("info"));

    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        // Весь вывод проходит через вырезание секретов перед записью.
        .with_writer(RedactingMakeWriter::new(std::io::stderr))
        .with_target(false)
        .try_init();
}
