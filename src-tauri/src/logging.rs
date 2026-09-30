//! Setting up log output.
//!
//! The one point where the log meets the output stream — and therefore the one place
//! to put secret redaction so that it covers everything (constitution, principle IV).
//!
//! ⚠ **T655 (QA-24A №6) — the log goes to a file as well as to stderr.** A Windows release is
//! built without a console (`main.rs`), so stderr there goes nowhere: a start-up that failed
//! left nothing a person could find or send. The file lives in the application's data
//! directory, under `logs/`, and is kept to a size: when it would outgrow [`MAX_BYTES`] it
//! becomes `vrcast-studio.log.1` (the one before that is dropped) and a new one begins.
//! Both outputs pass through the same redaction.

use crate::store::redact::RedactingMakeWriter;
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

/// The name of the log file inside its directory.
pub const FILE_NAME: &str = "vrcast-studio.log";

/// How large one log file may grow before it is set aside: 5 MiB. Days of ordinary use
/// fit, and the two files together stay small enough to attach to a message.
pub const MAX_BYTES: u64 = 5 * 1024 * 1024;

/// How many set-aside files are kept beside the current one.
pub const KEEP: usize = 1;

/// Where this run's log is being written, or why it is not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Journal {
    /// Written to this file (as well as to stderr).
    File(PathBuf),
    /// Only to stderr; the reason the file could not be had.
    NotWritten(String),
}

impl Journal {
    pub fn path(&self) -> Option<&Path> {
        match self {
            Journal::File(p) => Some(p),
            Journal::NotWritten(_) => None,
        }
    }
}

/// The directory the log goes into: `logs/` beside the local database, in the application's
/// data directory.
pub fn default_dir() -> Option<PathBuf> {
    crate::store::db::Db::default_path()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("logs")))
}

/// A log file that sets itself aside when it grows too large.
///
/// Each write arrives as whole lines (the redacting writer in front of it gathers them) and
/// goes in under one lock — so lines from two threads do not interleave and a rotation never
/// splits one.
pub struct LogFile {
    path: PathBuf,
    max_bytes: u64,
    keep: usize,
    state: Mutex<Open>,
}

struct Open {
    file: Option<File>,
    written: u64,
}

impl LogFile {
    /// Open (or create) the log in `dir` with the standard limits.
    pub fn open(dir: &Path) -> io::Result<Arc<Self>> {
        Self::open_with(dir, MAX_BYTES, KEEP)
    }

    /// The same, with the limits given — for tests.
    pub fn open_with(dir: &Path, max_bytes: u64, keep: usize) -> io::Result<Arc<Self>> {
        std::fs::create_dir_all(dir)?;
        let path = dir.join(FILE_NAME);
        let file = open_append(&path)?;
        let written = file.metadata().map(|m| m.len()).unwrap_or(0);
        let log = Self {
            path,
            max_bytes: max_bytes.max(1),
            keep,
            state: Mutex::new(Open {
                file: Some(file),
                written,
            }),
        };
        // A file already at the limit from an earlier run is set aside before the first line
        // of this one, rather than let grow further.
        if written >= log.max_bytes {
            let mut st = log.lock();
            log.rotate(&mut st)?;
        }
        Ok(Arc::new(log))
    }

    /// The path of the current file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Open> {
        // A panic mid-write poisons nothing that matters here: at worst one torn line.
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// `name.log` → `name.log.1` → … → `name.log.{keep}`; the oldest is dropped.
    fn rotate(&self, st: &mut Open) -> io::Result<()> {
        // Closed first: on Windows an open file cannot be renamed.
        st.file = None;
        if self.keep == 0 {
            let _ = std::fs::remove_file(&self.path);
        } else {
            let _ = std::fs::remove_file(self.aside(self.keep));
            for n in (1..self.keep).rev() {
                let _ = std::fs::rename(self.aside(n), self.aside(n + 1));
            }
            let _ = std::fs::rename(&self.path, self.aside(1));
        }
        let file = open_append(&self.path)?;
        st.written = file.metadata().map(|m| m.len()).unwrap_or(0);
        st.file = Some(file);
        Ok(())
    }

    fn aside(&self, n: usize) -> PathBuf {
        let mut name = self.path.as_os_str().to_owned();
        name.push(format!(".{n}"));
        PathBuf::from(name)
    }

    fn write_lines(&self, buf: &[u8]) -> io::Result<()> {
        let mut st = self.lock();
        if st.written > 0 && st.written + buf.len() as u64 > self.max_bytes {
            self.rotate(&mut st)?;
        }
        let file = match st.file.take() {
            Some(f) => f,
            // A rotation that failed half-way left no file: try once more to have one.
            None => open_append(&self.path)?,
        };
        let file = st.file.insert(file);
        file.write_all(buf)?;
        st.written += buf.len() as u64;
        Ok(())
    }
}

/// Open for appending.
///
/// **On Windows, shared for deletion as well.** "Forget everything" (FR-114) removes the whole
/// data directory while this file is open; without `FILE_SHARE_DELETE` the removal would fail
/// on the log and the directory would stay behind.
fn open_append(path: &Path) -> io::Result<File> {
    let mut o = OpenOptions::new();
    o.create(true).append(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE
        o.share_mode(0x1 | 0x2 | 0x4);
    }
    o.open(path)
}

/// The writer handed to `tracing` for one event.
pub struct LogFileWriter(Arc<LogFile>);

impl Write for LogFileWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write_lines(buf)?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Hands out writers into one shared [`LogFile`].
#[derive(Clone)]
pub struct MakeLogFileWriter(pub Arc<LogFile>);

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for MakeLogFileWriter {
    type Writer = LogFileWriter;

    fn make_writer(&'a self) -> Self::Writer {
        LogFileWriter(self.0.clone())
    }
}

fn filter() -> EnvFilter {
    EnvFilter::try_from_env("VRCAST_LOG").unwrap_or_else(|_| EnvFilter::new("info"))
}

/// The subscriber: stderr as before, and the file when there is one. Both redacted.
///
/// Public so that a test can install it for one closure (`tracing::subscriber::with_default`)
/// rather than for the whole process.
pub fn subscriber(file: Option<Arc<LogFile>>) -> impl tracing::Subscriber + Send + Sync {
    subscriber_with(true, file)
}

/// The same, with stderr optional — a test that fills a log file does not need to fill the
/// test output with it too.
pub fn subscriber_with(
    to_stderr: bool,
    file: Option<Arc<LogFile>>,
) -> impl tracing::Subscriber + Send + Sync {
    let stderr = to_stderr.then(|| {
        tracing_subscriber::fmt::layer()
            // All output passes through secret redaction before it is written.
            .with_writer(RedactingMakeWriter::new(std::io::stderr))
            .with_target(false)
    });
    let file = file.map(|f| {
        tracing_subscriber::fmt::layer()
            .with_writer(RedactingMakeWriter::new(MakeLogFileWriter(f)))
            .with_target(false)
            // Colour codes are for a terminal; in a file they are noise.
            .with_ansi(false)
    });
    tracing_subscriber::registry()
        .with(filter())
        .with(stderr)
        .with(file)
}

/// Turn logging on — stderr only. For tests and tools.
///
/// Calling it again breaks nothing: installing the global subscriber quietly fails and
/// the arrangement already in force stays.
pub fn init() {
    let _ = subscriber(None).try_init();
}

/// Turn logging on for the application: stderr and the file in `dir`. Called once, when the
/// application starts.
///
/// A log file that cannot be had is not a reason to refuse to start: the answer says why, and
/// the start-up failure window, if it comes to one, repeats it.
pub fn init_for_app(dir: Option<PathBuf>) -> Journal {
    let (file, journal) = open_journal(dir);
    let _ = subscriber(file).try_init();
    journal
}

/// Open the log file in `dir`, and say where it is or why it is not. Split out of
/// [`init_for_app`] so it can be checked without installing a process-wide subscriber.
pub fn open_journal(dir: Option<PathBuf>) -> (Option<Arc<LogFile>>, Journal) {
    match dir {
        None => (
            None,
            Journal::NotWritten("the application data directory is unknown".into()),
        ),
        Some(dir) => match LogFile::open(&dir) {
            Ok(f) => {
                let path = f.path().to_path_buf();
                (Some(f), Journal::File(path))
            }
            Err(e) => (
                None,
                Journal::NotWritten(format!("{}: {e}", dir.join(FILE_NAME).display())),
            ),
        },
    }
}
