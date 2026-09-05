//! T173 — the application's settings.
//!
//! What the person may change and what the core has to be told about. Kept apart from the
//! server profiles on purpose: these belong to the application rather than to any one
//! server, and switching servers must not change them.
//!
//! **No secrets here.** Passphrases and keys live in the operating system's own store
//! (constitution, principle IV); this is an ordinary file in a person's profile, and
//! anything put in it is readable by anything that can read that profile.

use crate::domain::viewers::DEFAULT_ACTIVITY_THRESHOLD_S;
use crate::store::db::{Db, DbError};
use rusqlite::OptionalExtension;

/// Everything that can be set.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Settings {
    /// How long after their last sign of life a viewer stops being counted as watching
    /// (FR-055).
    pub viewer_activity_threshold_s: u64,
    /// Whether an outside service may be asked to place an address more exactly.
    ///
    /// **Off, and it takes a deliberate act to turn on** (FR-057). Asking means handing a
    /// viewer's address to somebody else — for every viewer, every session. That is a
    /// decision for the person whose friends are watching, not a default someone else
    /// chose for them.
    pub geo_refine_outside: bool,
    /// How many heavy tasks may run at once.
    pub concurrent_heavy_tasks: u32,
    /// Whether the mascot is shown, and whether things move.
    pub mascot: bool,
    pub animations: bool,
    /// Which of the interface's languages to use. `None` means "the system's".
    pub language: Option<String>,
    /// Light or dark. `None` means "the system's".
    pub theme: Option<String>,
    /// Whether the close button hides the window instead of ending the application (FR-150).
    ///
    /// **A preference, and only half of the decision.** The other half is whether there is
    /// anywhere to hide to: on a desktop with no tray the button closes whatever this says,
    /// because a window hidden into nothing is the worst outcome available — the application
    /// goes on holding encodes with nothing on screen to say so.
    pub close_to_tray: bool,
    /// Whether the person has already been told where the window goes (T399).
    ///
    /// ⚠ **A remembered fact, not a preference**, and it is here because this is where facts
    /// that must survive a restart live. It exists because the thing it stands in for cannot
    /// be checked: `rect()` on Linux is always `None`, there are no tray events and no error,
    /// so "the icon is invisible" is indistinguishable from "the icon is there" (R-35). On
    /// Windows 11 a new icon goes into the overflow, which is invisible in a different way.
    /// Either way the window vanishes and the only honest answer is to say where it went —
    /// once, because saying it every time is how people learn to dismiss notices unread.
    pub tray_notice_seen: bool,
    /// Where a variant is written while it is being made (T450, T451).
    ///
    /// `None` means "beside the source" — see `domain::work_dir`, which holds the reasoning
    /// and the fallback. Stored rather than derived so that somebody with a scratch disk can
    /// say so once instead of on every build.
    pub work_dir: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            viewer_activity_threshold_s: DEFAULT_ACTIVITY_THRESHOLD_S,
            geo_refine_outside: false,
            concurrent_heavy_tasks: 1,
            mascot: true,
            animations: true,
            language: None,
            theme: None,
            close_to_tray: true,
            tray_notice_seen: false,
            work_dir: None,
        }
    }
}

/// The bounds a setting is held to.
///
/// Not decoration: a threshold of zero would empty the list of viewers the instant anyone
/// paused, and one of a day would keep yesterday's viewers in it. The bounds are wide —
/// this is about keeping a value usable, not about second-guessing the person.
pub const MIN_THRESHOLD_S: u64 = 5;
pub const MAX_THRESHOLD_S: u64 = 600;
pub const MAX_HEAVY_TASKS: u32 = 8;

impl Settings {
    /// Bring the values inside their bounds.
    ///
    /// Clamped rather than refused. A setting out of range comes from an older version of
    /// the application or from somebody editing the file by hand, and refusing to start
    /// over it would be out of all proportion to the harm.
    pub fn clamped(mut self) -> Self {
        self.viewer_activity_threshold_s = self
            .viewer_activity_threshold_s
            .clamp(MIN_THRESHOLD_S, MAX_THRESHOLD_S);
        self.concurrent_heavy_tasks = self.concurrent_heavy_tasks.clamp(1, MAX_HEAVY_TASKS);
        self
    }

    /// The viewer threshold as a span.
    pub fn activity_threshold(&self) -> time::Duration {
        time::Duration::seconds(self.viewer_activity_threshold_s as i64)
    }
}

/// Read the settings.
///
/// Anything absent takes its default, and anything unreadable takes its default too. The
/// settings are a convenience; refusing to start because one of them will not parse would
/// turn a convenience into a way of locking a person out of their own application.
pub fn load(db: &Db) -> Result<Settings, DbError> {
    let mut settings = Settings::default();
    db.with_conn(|conn| {
        let mut stmt = conn.prepare("SELECT name, value FROM settings")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        for row in rows {
            let (name, value) = row?;
            apply(&mut settings, &name, &value);
        }
        Ok(())
    })?;
    Ok(settings.clamped())
}

/// The key a viewer's pseudonym is made with (T222).
///
/// Made once, on this machine, and kept: the pseudonym has to be the same after the
/// application is closed and opened again, or one viewer becomes a different stranger
/// every session and the whole point of it goes.
///
/// Kept beside the settings rather than among the secrets: it is not a secret in the
/// sense principle IV means — losing it costs the ability to compare old log lines with
/// new ones, and nothing else. What it must never be is **shared**, and a value that
/// never leaves this machine is not.
pub fn pseudonym_key(db: &Db) -> Result<String, DbError> {
    const NAME: &str = "pseudonym_key";

    let existing: Option<String> = db.with_conn(|c| {
        Ok(
            c.query_row("SELECT value FROM settings WHERE name = ?1", [NAME], |r| {
                r.get(0)
            })
            .optional()?,
        )
    })?;
    if let Some(key) = existing.filter(|k| !k.is_empty()) {
        return Ok(key);
    }

    let key = uuid::Uuid::new_v4().simple().to_string();
    db.with_conn(|c| {
        c.execute(
            "INSERT INTO settings (name, value) VALUES (?1, ?2)
             ON CONFLICT (name) DO UPDATE SET value = excluded.value",
            rusqlite::params![NAME, key],
        )?;
        Ok(())
    })?;
    Ok(key)
}

/// Write the settings, replacing what was there.
pub fn save(db: &Db, settings: &Settings) -> Result<Settings, DbError> {
    let settings = settings.clone().clamped();
    let pairs = [
        (
            "viewer_activity_threshold_s",
            settings.viewer_activity_threshold_s.to_string(),
        ),
        (
            "geo_refine_outside",
            settings.geo_refine_outside.to_string(),
        ),
        (
            "concurrent_heavy_tasks",
            settings.concurrent_heavy_tasks.to_string(),
        ),
        ("mascot", settings.mascot.to_string()),
        ("animations", settings.animations.to_string()),
        ("language", settings.language.clone().unwrap_or_default()),
        ("theme", settings.theme.clone().unwrap_or_default()),
        ("close_to_tray", settings.close_to_tray.to_string()),
        ("tray_notice_seen", settings.tray_notice_seen.to_string()),
        ("work_dir", settings.work_dir.clone().unwrap_or_default()),
    ];
    db.with_conn_mut(|conn| {
        let tx = conn.transaction()?;
        for (name, value) in &pairs {
            tx.execute(
                "INSERT INTO settings (name, value) VALUES (?1, ?2)
                 ON CONFLICT(name) DO UPDATE SET value = excluded.value",
                rusqlite::params![name, value],
            )?;
        }
        tx.commit()?;
        Ok(())
    })?;
    Ok(settings)
}

/// Put one stored value into place.
///
/// A value that will not parse leaves the default standing. Silently: the alternative is a
/// warning on every start for a setting nobody can see is broken.
fn apply(settings: &mut Settings, name: &str, value: &str) {
    match name {
        "viewer_activity_threshold_s" => {
            if let Ok(v) = value.parse() {
                settings.viewer_activity_threshold_s = v;
            }
        }
        "geo_refine_outside" => {
            if let Ok(v) = value.parse() {
                settings.geo_refine_outside = v;
            }
        }
        "concurrent_heavy_tasks" => {
            if let Ok(v) = value.parse() {
                settings.concurrent_heavy_tasks = v;
            }
        }
        "mascot" => {
            if let Ok(v) = value.parse() {
                settings.mascot = v;
            }
        }
        "close_to_tray" => {
            if let Ok(v) = value.parse() {
                settings.close_to_tray = v;
            }
        }
        "tray_notice_seen" => {
            if let Ok(v) = value.parse() {
                settings.tray_notice_seen = v;
            }
        }
        "animations" => {
            if let Ok(v) = value.parse() {
                settings.animations = v;
            }
        }
        // An empty string means "not chosen": the system decides. Storing it as a real
        // value would make "the system's" indistinguishable from a language named "".
        "language" => settings.language = (!value.is_empty()).then(|| value.to_owned()),
        "theme" => settings.theme = (!value.is_empty()).then(|| value.to_owned()),
        // Empty means "not chosen" here for the same reason as above, and it matters more:
        // a blank stored as a real value would put two gigabytes in a folder named nothing.
        "work_dir" => settings.work_dir = (!value.is_empty()).then(|| value.to_owned()),
        _ => {}
    }
}
