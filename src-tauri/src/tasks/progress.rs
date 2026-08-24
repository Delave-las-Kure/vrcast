//! T020 — ограничение частоты событий прогресса.
//!
//! Передача файла сообщает о продвижении сотни раз в секунду. Если пропускать всё,
//! поток событий сам станет причиной подтормаживания интерфейса — то есть средство
//! показать отзывчивость её же и убьёт (SC-009, R-15).
//!
//! Ключевая тонкость не в ограничении, а в исключениях из него. Последнее сообщение
//! перед завершением обязано пройти всегда, иначе полоса застынет на 87 % у задачи,
//! которая уже закончилась.

use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Не чаще четырёх раз в секунду на задачу.
pub const MIN_INTERVAL: Duration = Duration::from_millis(250);

/// Пропускной клапан для событий прогресса.
#[derive(Debug)]
pub struct ProgressThrottle {
    min_interval: Duration,
    last: Mutex<Option<Instant>>,
}

impl Default for ProgressThrottle {
    fn default() -> Self {
        Self::new(MIN_INTERVAL)
    }
}

impl ProgressThrottle {
    pub fn new(min_interval: Duration) -> Self {
        Self {
            min_interval,
            last: Mutex::new(None),
        }
    }

    /// Пропустить ли это событие.
    ///
    /// `important` — сообщение, которое обязано пройти независимо от частоты: смена
    /// состояния, завершение, ошибка. Без этого исключения показатель застревает
    /// на последнем пропущенном значении.
    pub fn allow(&self, important: bool) -> bool {
        self.allow_at(Instant::now(), important)
    }

    /// То же, но с явным моментом времени — чтобы тесты не зависели от настоящих часов.
    pub fn allow_at(&self, now: Instant, important: bool) -> bool {
        let mut last = match self.last.lock() {
            Ok(l) => l,
            // Отравленная блокировка не повод терять событие.
            Err(e) => e.into_inner(),
        };

        if important {
            *last = Some(now);
            return true;
        }

        match *last {
            Some(prev) if now.duration_since(prev) < self.min_interval => false,
            _ => {
                *last = Some(now);
                true
            }
        }
    }

    /// Забыть отметку — например, когда задача продолжается после приостановки.
    pub fn reset(&self) {
        if let Ok(mut last) = self.last.lock() {
            *last = None;
        }
    }
}
