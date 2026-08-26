//! The kinds of task, their states, and the transitions that are allowed.
//!
//! There is neither input-output nor networking here — only rules. That is deliberate
//! (constitution, section "Limits on quality of execution"): logic that can only be checked
//! through a database or a server counts as unchecked.

use serde::{Deserialize, Serialize};

/// Declares an enumeration from ONE list: the enum, `ALL`, `as_str` and `parse` are all
/// born from it. A hand-written list beside an enum is a loophole: a code added to the enum
/// and forgotten in the list drops out of the comparison against the TypeScript contract,
/// and the compiler says nothing about it. The same trick is used in `commands::error`.
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
            /// Every variant. Born of the same list as the enum — they cannot diverge.
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
    /// The kind of a task.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum TaskKind {
        /// Examining a source file — quick, and local.
        Probe => "probe",
        /// Preparing a file for serving.
        Convert => "convert",
        /// Sending a file to the server.
        Upload => "upload",
        /// Measuring what each rung is worth on this material — the longest of them all.
        MeasureQuality => "measure_quality",
        /// Building a quality ladder on the server.
        BuildLadder => "build_ladder",
        /// Setting up serving on a bare server.
        Deploy => "deploy",
        /// Upgrading the server side.
        UpgradeServer => "upgrade_server",
        /// Taking a reading of the server's state.
        Diagnose => "diagnose",
    }
}

impl TaskKind {
    /// Which resource a task takes up.
    pub fn lane(&self) -> Lane {
        match self {
            Self::Convert | Self::MeasureQuality => Lane::Compute,
            Self::Upload | Self::BuildLadder | Self::Deploy | Self::UpgradeServer => Lane::Network,
            Self::Probe | Self::Diagnose => Lane::Light,
        }
    }

    /// Whether it can be paused without losing the work, and whether that survives the
    /// application being closed.
    pub fn pause_kind(&self) -> PauseKind {
        match self {
            // The position is held as bytes on the server: it carries on even after a
            // restart (R-05).
            Self::Upload => PauseKind::ResumableAcrossRestart,
            // A suspended process lives only as long as the application does (the owner's
            // decision, 2026-08-24). Closing the application loses the work done — and a
            // person must learn that BEFORE closing it (FR-086).
            Self::Convert => PauseKind::SuspendedProcess,
            // Assembled from completed steps: it carries on from what is already done.
            //
            // A measurement belongs here rather than with preparing a file: each point of
            // the grid is written down as it is taken, so what survives a restart is not an
            // estimate but the exact points that answered.
            Self::BuildLadder | Self::Deploy | Self::UpgradeServer | Self::MeasureQuality => {
                PauseKind::ResumableAcrossRestart
            }
            // Short ones: there is nothing to pause, running them again is simpler.
            Self::Probe | Self::Diagnose => PauseKind::NotPausable,
        }
    }
}

/// A lane — the resource tasks compete with one another for.
///
/// One shared limit over all tasks would be wrong: preparing a file is bound by computation
/// and a transfer by the network, and forbidding them to run at the same time makes no
/// sense. Two preparations at once, on the other hand, are each twice as slow and win
/// nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Lane {
    /// Computation: preparing a file.
    Compute,
    /// The network and the server: a transfer, a build on the server, a setup.
    Network,
    /// Short checks: they take up almost nothing.
    Light,
}

/// How a task takes being paused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PauseKind {
    /// Carries on from where it got to, even after the application restarts.
    ResumableAcrossRestart,
    /// A suspended process holds the work, but will not survive the application closing.
    SuspendedProcess,
    /// Pausing is not supported.
    NotPausable,
}

str_enum! {
    /// The state of a task.
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
    /// The finished states: there are no transitions out of them.
    pub fn is_final(&self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }

    /// Whether the task takes up a place in its lane.
    pub fn occupies_lane(&self) -> bool {
        // A paused preparation holds its process in memory but does no computing — its
        // place in the lane is freed, or pausing would gain nothing at all.
        matches!(self, Self::Running)
    }

    /// Whether a transition is allowed. The one place where that is decided.
    pub fn can_transition_to(&self, next: TaskState) -> bool {
        use TaskState::*;
        match (self, next) {
            (Queued, Running | Cancelled) => true,
            (Running, Completed | Failed | Paused | Cancelled) => true,
            // Out of paused — not only carrying on and cancelling, but finishing too.
            // A pause takes effect at the nearest stopping point, and the work can run to
            // its end while the task is already marked paused: a transfer finishes writing
            // its last window, a preparation its last step. Refusing to record that would
            // be lying about a finished result; the table used to forbid this transition
            // while the engine made it anyway, and the disagreement said nothing
            // (debt T072).
            (Paused, Running | Cancelled | Completed | Failed) => true,
            // A transition into itself is allowed: pressing cancel a second time must not
            // be an error (constitution, principle V).
            (a, b) if a == &b => true,
            _ => false,
        }
    }
}

/// The limits on simultaneous tasks, per lane.
#[derive(Debug, Clone, Copy)]
pub struct LaneLimits {
    pub compute: usize,
    pub network: usize,
    pub light: usize,
}

impl Default for LaneLimits {
    fn default() -> Self {
        Self {
            // Two preparations at once are each twice as slow — nothing is gained.
            compute: 1,
            // Two transfers share one network link; besides, a server limits how many
            // connections may be established at once (R-04).
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
