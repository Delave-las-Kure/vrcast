//! Виды задач, их состояния и разрешённые переходы.
//!
//! Здесь нет ни ввода-вывода, ни сети — только правила. Это сделано намеренно
//! (конституция, раздел «Ограничения качества исполнения»): логика, которую можно
//! проверить только через базу или сервер, считается непроверенной.

use serde::{Deserialize, Serialize};

/// Объявляет перечисление ОДНИМ перечнем: из него рождаются и enum, и `ALL`,
/// и `as_str`, и `parse`. Рукописный список рядом с enum — лазейка: код,
/// добавленный в enum и забытый в списке, выпадает из сверки с TS-договором,
/// и компилятор при этом молчит. Тот же приём — в `commands::error`.
macro_rules! str_enum {
    (
        $(#[$outer:meta])*
        $vis:vis enum $enum_name:ident {
            $($(#[$meta:meta])* $name:ident => $code:literal),+ $(,)?
        }
    ) => {
        $(#[$outer])*
        $vis enum $enum_name {
            $($(#[$meta])* $name,)+
        }

        impl $enum_name {
            /// Все варианты. Порождён тем же перечнем, что и enum, — разойтись не могут.
            pub const ALL: &'static [$enum_name] = &[$(Self::$name),+];

            pub fn as_str(&self) -> &'static str {
                match self { $(Self::$name => $code,)+ }
            }

            pub fn parse(s: &str) -> Option<Self> {
                match s {
                    $($code => Some(Self::$name),)+
                    _ => None,
                }
            }
        }
    };
}

str_enum! {
    /// Вид задачи.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum TaskKind {
        /// Разбор исходника — быстро, локально.
        Probe => "probe",
        /// Подготовка файла к раздаче.
        Convert => "convert",
        /// Передача файла на сервер.
        Upload => "upload",
        /// Сборка набора качеств на сервере.
        BuildLadder => "build_ladder",
        /// Развёртывание раздачи на чистом сервере.
        Deploy => "deploy",
        /// Обновление серверной части.
        UpgradeServer => "upgrade_server",
        /// Снятие состояния сервера.
        Diagnose => "diagnose",
    }
}

impl TaskKind {
    /// Какой ресурс занимает задача.
    pub fn lane(&self) -> Lane {
        match self {
            Self::Convert => Lane::Compute,
            Self::Upload | Self::BuildLadder | Self::Deploy | Self::UpgradeServer => Lane::Network,
            Self::Probe | Self::Diagnose => Lane::Light,
        }
    }

    /// Можно ли приостановить, не потеряв работу, и переживёт ли это закрытие приложения.
    pub fn pause_kind(&self) -> PauseKind {
        match self {
            // Позиция хранится в байтах на сервере: продолжится и после перезапуска (R-05).
            Self::Upload => PauseKind::ResumableAcrossRestart,
            // Приостановленный процесс живёт, только пока живо приложение (решение владельца
            // 2026-08-24). Закрытие приложения теряет проделанную работу — и пользователь
            // обязан узнать об этом ДО закрытия (FR-086).
            Self::Convert => PauseKind::SuspendedProcess,
            // Собирается из выполненных шагов: продолжится с того, что уже готово.
            Self::BuildLadder | Self::Deploy | Self::UpgradeServer => {
                PauseKind::ResumableAcrossRestart
            }
            // Короткие: приостанавливать нечего, проще выполнить заново.
            Self::Probe | Self::Diagnose => PauseKind::NotPausable,
        }
    }
}

/// Полоса — по какому ресурсу задачи конкурируют между собой.
///
/// Общий предел на все задачи был бы неверен: подготовка файла упирается в вычисления,
/// передача — в канал, и запрещать им идти одновременно бессмысленно. А вот две подготовки
/// сразу вдвое медленнее каждая и ничего не выигрывают.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Lane {
    /// Вычисления: подготовка файла.
    Compute,
    /// Канал и сервер: передача, сборка на сервере, развёртывание.
    Network,
    /// Короткие проверки: почти ничего не занимают.
    Light,
}

/// Как задача переносит приостановку.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PauseKind {
    /// Продолжится с достигнутого места даже после перезапуска приложения.
    ResumableAcrossRestart,
    /// Приостановленный процесс держит работу, но не переживёт закрытия приложения.
    SuspendedProcess,
    /// Приостановка не поддерживается.
    NotPausable,
}

str_enum! {
    /// Состояние задачи.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum TaskState {
        Queued => "queued",
        Running => "running",
        Paused => "paused",
        Completed => "completed",
        Failed => "failed",
        Cancelled => "cancelled",
    }
}

impl TaskState {
    /// Завершённые состояния: из них переходов нет.
    pub fn is_final(&self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }

    /// Занимает ли задача место в полосе.
    pub fn occupies_lane(&self) -> bool {
        // Приостановленная подготовка держит процесс в памяти, но вычислений не ведёт —
        // место в полосе освобождается, иначе приостановка не давала бы ничего.
        matches!(self, Self::Running)
    }

    /// Разрешён ли переход. Единственное место, где это решается.
    pub fn can_transition_to(&self, next: TaskState) -> bool {
        use TaskState::*;
        match (self, next) {
            (Queued, Running | Cancelled) => true,
            (Running, Completed | Failed | Paused | Cancelled) => true,
            (Paused, Running | Cancelled) => true,
            // Переход в самого себя допустим: повторное нажатие отмены не должно быть
            // ошибкой (конституция, принцип V).
            (a, b) if a == &b => true,
            _ => false,
        }
    }
}

/// Пределы одновременных задач по полосам.
#[derive(Debug, Clone, Copy)]
pub struct LaneLimits {
    pub compute: usize,
    pub network: usize,
    pub light: usize,
}

impl Default for LaneLimits {
    fn default() -> Self {
        Self {
            // Две подготовки сразу вдвое медленнее каждая — выигрыша нет.
            compute: 1,
            // Две передачи делят один канал; кроме того, сервер ограничивает число
            // одновременно устанавливаемых соединений (R-04).
            network: 1,
            light: 4,
        }
    }
}

impl LaneLimits {
    pub fn for_lane(&self, lane: Lane) -> usize {
        match lane {
            Lane::Compute => self.compute,
            Lane::Network => self.network,
            Lane::Light => self.light,
        }
    }
}
