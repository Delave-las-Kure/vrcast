//! T079 — скорость и оставшееся время (FR-035).
//!
//! Показывать мгновенную скорость нельзя: она скачет от окна к окну, и число
//! в интерфейсе мельтешит так, что прочитать его невозможно. Показывать среднюю
//! за всё время тоже нельзя: после обрыва и получаса простоя она покажет вдвое
//! меньше, чем идёт на самом деле.
//!
//! Поэтому скорость считается по скользящему окну последних секунд. И отдельно —
//! правило про паузы: если между двумя отсчётами прошло больше окна, накопленное
//! больше не описывает происходящее и выбрасывается. Без этого правила после паузы
//! пользователь видит «осталось четыреста часов» и решает, что всё сломалось.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// За какой отрезок усредняем.
const WINDOW: Duration = Duration::from_secs(10);

/// Сколько отсчётов держим. Больше не нужно: при четырёх событиях в секунду
/// (R-15) их и так не наберётся сверх этого за окно усреднения.
const MAX_SAMPLES: usize = 64;

#[derive(Debug)]
pub struct ProgressEstimate {
    /// Пары «когда» и «сколько передано всего».
    samples: VecDeque<(Instant, u64)>,
    window: Duration,
}

impl Default for ProgressEstimate {
    fn default() -> Self {
        Self::new(WINDOW)
    }
}

impl ProgressEstimate {
    pub fn new(window: Duration) -> Self {
        Self {
            samples: VecDeque::new(),
            window,
        }
    }

    /// Записать, сколько всего передано к этому мгновению.
    pub fn record(&mut self, now: Instant, transferred: u64) {
        // Разрыв длиннее окна усреднения — это пауза, обрыв или перезапуск.
        // Накопленное до него ничего не говорит о нынешней скорости.
        if let Some((last, _)) = self.samples.back() {
            if now.saturating_duration_since(*last) > self.window {
                self.samples.clear();
            }
        }

        self.samples.push_back((now, transferred));

        // Выбрасываем всё, что старше окна, но последний отсчёт бережём: без него
        // не с чем сравнивать следующий.
        while self.samples.len() > 1 {
            let Some((oldest, _)) = self.samples.front() else {
                break;
            };
            if now.saturating_duration_since(*oldest) > self.window
                || self.samples.len() > MAX_SAMPLES
            {
                self.samples.pop_front();
            } else {
                break;
            }
        }
    }

    /// Скорость в байтах в секунду. `None`, пока отсчётов не хватает для вывода.
    pub fn speed_bps(&self) -> Option<u64> {
        let (first_at, first_bytes) = *self.samples.front()?;
        let (last_at, last_bytes) = *self.samples.back()?;

        let seconds = last_at.saturating_duration_since(first_at).as_secs_f64();
        // Слишком короткий отрезок даёт число, которому нельзя верить: деление
        // на тысячные доли секунды превращает любую дрожь в гигабиты.
        if seconds < 0.5 {
            return None;
        }
        let bytes = last_bytes.saturating_sub(first_bytes);
        Some((bytes as f64 / seconds).round() as u64)
    }

    /// Сколько осталось при нынешней скорости. `None`, если скорость неизвестна
    /// или равна нулю — «бесконечность» показывать человеку незачем.
    pub fn eta(&self, remaining: u64) -> Option<Duration> {
        let speed = self.speed_bps()?;
        if speed == 0 {
            return None;
        }
        Some(Duration::from_secs_f64(remaining as f64 / speed as f64))
    }

    /// Забыть накопленное — при паузе, обрыве и продолжении после перезапуска.
    pub fn reset(&mut self) {
        self.samples.clear();
    }
}
