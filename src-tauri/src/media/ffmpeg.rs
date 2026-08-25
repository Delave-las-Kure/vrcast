//! T115 — the FFmpeg bundled with the application: where it lies and what it can do
//! (FR-112, R-01).
//!
//! The application downloads nothing on its first run. A person installs it and prepares a
//! video straight away; the FFmpeg build is put beside it when the installer is built
//! (`scripts/fetch-ffmpeg.mjs`, with the exact snapshot pinned in `scripts/ffmpeg.json`).
//!
//! **Checking at start-up is not optional.** The bundled file may fail to run: an antivirus
//! cut it out, the installer unpacked only halfway, the file has no execute permission.
//! Learning that at the start means telling a person what to fix. Learning it halfway
//! through a two-hour preparation means taking those two hours away.
//!
//! Examining sources and running preparations are not here: those are `probe.rs` and
//! `convert.rs`. Here there is only "is there anything to work with at all".

use std::path::{Path, PathBuf};
use std::process::Stdio;

#[derive(Debug, thiserror::Error)]
pub enum FfmpegError {
    #[error("bundled FFmpeg not found: looked in {0}")]
    NotFound(String),

    #[error("bundled FFmpeg will not start: {0}")]
    NotRunnable(String),

    #[error("bundled FFmpeg answers with something other than it should: {0}")]
    Unexpected(String),
}

pub type Result<T> = std::result::Result<T, FfmpegError>;

/// What could be learned about the bundled build.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FfmpegInfo {
    /// The version string as the program itself gives it.
    pub version: String,
    /// The full path — needed when sorting out trouble.
    pub path: String,
    /// Whether there is a software H.264 encoder. Without it no preparation is possible.
    pub has_x264: bool,
    /// The hardware H.264 encoders this build knows how to call.
    ///
    /// "Knows how to call" is not "work here": their presence in the build says nothing
    /// about the graphics card. The real check is a trial run, and it is separate
    /// (FR-026).
    pub hardware: Vec<String>,
}

/// The hardware H.264 encoders we care about.
///
/// The order is a preference: NVIDIA is faster than the rest on our material, then Intel's
/// built into the processor, then AMD, and last the general Linux path.
///
/// `h264_vaapi` must be in the list even though it is useless on Windows: on Linux it is
/// what both Intel and AMD work through. Without it, half of all Linux machines simply
/// would not find their hardware acceleration — while having it.
///
/// The build carries others too (`h264_mf`, `h264_d3d12va`, `h264_vulkan`), but they are
/// either wrappers over the same ones or untested on our material; declaring available
/// something we have not tried is a promise with nobody to answer for it.
const HW_ENCODERS: [&str; 4] = ["h264_nvenc", "h264_qsv", "h264_amf", "h264_vaapi"];

/// The name of a bundled program beside the application.
///
/// The bundler puts bundled programs next to the executable, cutting the platform triple
/// off the name: `ffmpeg-x86_64-pc-windows-msvc.exe` becomes `ffmpeg.exe`.
fn bundled_name(tool: &str) -> String {
    format!("{tool}{}", std::env::consts::EXE_SUFFIX)
}

/// Find a bundled program.
///
/// First beside the application — that is where an installed application keeps it. A debug
/// build adds the directory `npm run ffmpeg` puts it in: otherwise development and tests
/// would need the installer built every time. A released application deliberately has no
/// such fallback — it would point at a directory on someone else's machine.
pub fn locate(tool: &str) -> Result<PathBuf> {
    let name = bundled_name(tool);
    let mut tried: Vec<String> = Vec::new();

    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let candidate = dir.join(&name);
            if candidate.is_file() {
                return Ok(candidate);
            }
            tried.push(candidate.display().to_string());
        }
    }

    #[cfg(debug_assertions)]
    {
        let dev = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("binaries")
            .join(format!(
                "{tool}-{}{}",
                target_triple(),
                std::env::consts::EXE_SUFFIX
            ));
        if dev.is_file() {
            return Ok(dev);
        }
        tried.push(dev.display().to_string());
    }

    Err(FfmpegError::NotFound(tried.join(", ")))
}

/// The platform triple the application was built for.
///
/// Put together from the same parts the bundler uses: there is no separate way to ask the
/// program itself for it.
#[cfg(debug_assertions)]
fn target_triple() -> String {
    let arch = std::env::consts::ARCH;
    match std::env::consts::OS {
        "windows" => format!("{arch}-pc-windows-msvc"),
        "linux" => format!("{arch}-unknown-linux-gnu"),
        "macos" => format!("{arch}-apple-darwin"),
        other => format!("{arch}-{other}"),
    }
}

/// Check the bundled build: does it start, and can it do what is needed.
pub async fn probe_self() -> Result<FfmpegInfo> {
    let path = locate("ffmpeg")?;
    // ffprobe is needed as much as ffmpeg: without it a source cannot be examined. Missing
    // either of the two is the same trouble, and it must be reported here rather than at
    // the first examination.
    locate("ffprobe")?;

    let version = parse_version(&run(&path, &["-hide_banner", "-version"]).await?)?;
    let encoders = run(&path, &["-hide_banner", "-encoders"]).await?;

    Ok(FfmpegInfo {
        version,
        path: path.display().to_string(),
        has_x264: encoder_present(&encoders, "libx264"),
        hardware: HW_ENCODERS
            .iter()
            .filter(|n| encoder_present(&encoders, n))
            .map(|n| (*n).to_owned())
            .collect(),
    })
}

/// Pull the version out of the program's answer.
///
/// Kept apart from running it so that the parsing is checked by a test without the bundled
/// file: continuous integration has none — there is no point downloading a hundred and
/// forty megabytes for every run — and without this split the parsing would go unchecked
/// entirely.
pub fn parse_version(text: &str) -> Result<String> {
    let line = first_line(text).ok_or_else(|| {
        FfmpegError::Unexpected(String::from("an empty answer to the version request"))
    })?;

    // The check is not pedantry: anything at all may stand behind the name `ffmpeg` in a
    // system — from a package manager's wrapper to a message saying the program is not
    // installed.
    if !line.starts_with("ffmpeg version") {
        return Err(FfmpegError::Unexpected(format!(
            "the version request was answered with \"{line}\""
        )));
    }
    Ok(line)
}

/// Whether such an encoder is in the listing.
///
/// **Only the name column** is looked at, not the whole listing. Searching the entire text
/// will not do, and that is not a theoretical quibble: in the line
/// `V....D h264_nvenc  NVIDIA NVENC H.264 encoder` the word "NVENC" also stands in the
/// human description, so a word search would declare the presence of an encoder called
/// `nvenc`, which does not exist. The application would decide hardware acceleration was
/// available and fall over as soon as a preparation started.
///
/// A substring search will do even less: `x264` would be found inside `libx264`, and `aac`
/// inside a good dozen names belonging to others.
pub fn encoder_present(listing: &str, name: &str) -> bool {
    encoder_names(listing).any(|n| n.eq_ignore_ascii_case(name))
}

/// The encoder names out of a listing.
///
/// A listing line looks like this: ` V....D libx264   libx264 H.264 / AVC`. The first
/// column is the properties (the kind of stream and what the encoder can do), the second is
/// the name, and after that comes a description for a person. A line is recognised by its
/// first column: it is exactly six characters long and made of a known set. The heading and
/// the explanations are then filtered out by themselves, without depending on their shape —
/// and their shape changes from version to version.
fn encoder_names(listing: &str) -> impl Iterator<Item = &str> {
    /// The characters the property column is made of.
    const FLAGS: &str = "VASFXBDIL.";

    listing.lines().filter_map(|line| {
        let mut parts = line.split_whitespace();
        let flags = parts.next()?;
        if flags.len() != 6 || !flags.chars().all(|c| FLAGS.contains(c)) {
            return None;
        }
        parts.next()
    })
}

async fn run(path: &Path, args: &[&str]) -> Result<String> {
    let out = tokio::process::Command::new(path)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .await
        .map_err(|e| FfmpegError::NotRunnable(format!("{}: {e}", path.display())))?;

    // `-encoders` and `-version` write to the ordinary output, but in some builds part of
    // what they say goes to the error stream. Both are taken: there is no point separating
    // them here.
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));

    if !out.status.success() && text.trim().is_empty() {
        return Err(FfmpegError::NotRunnable(format!(
            "{} exited with {} and said nothing",
            path.display(),
            out.status
        )));
    }
    Ok(text)
}

fn first_line(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(str::to_owned)
}
