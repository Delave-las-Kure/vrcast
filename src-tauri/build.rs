use std::path::PathBuf;
use std::process::Command;

/// One line of `git`, or nothing: a build from a source archive has no repository, and that is
/// no reason to refuse to build.
fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let line = String::from_utf8(out.stdout).ok()?.trim().to_owned();
    (!line.is_empty()).then_some(line)
}

/// Rebuild when this git file changes — and only then. `--git-path` answers relative to the
/// working directory (this package) or absolutely (a worktree), so both are resolved here.
fn watch(path: &str) {
    let path = PathBuf::from(path);
    let path = if path.is_absolute() {
        path
    } else {
        PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default()).join(path)
    };
    if path.exists() {
        println!("cargo:rerun-if-changed={}", path.display());
    }
}

fn main() {
    // Which build this is (owner, 2026-10-02: «нужно ввести версионирование, чтобы отмечать,
    // какая версия»). The number in Cargo.toml says which release; the commit and its date
    // say which build of it — two installers of the same number are told apart by these.
    if let Some(head) = git(&["rev-parse", "--git-path", "HEAD"]) {
        watch(&head);
    }
    if let Some(branch) = git(&["symbolic-ref", "-q", "HEAD"]) {
        if let Some(path) = git(&["rev-parse", "--git-path", &branch]) {
            watch(&path);
        }
    }
    if let Some(packed) = git(&["rev-parse", "--git-path", "packed-refs"]) {
        watch(&packed);
    }
    let commit = git(&["rev-parse", "--short=7", "HEAD"]).unwrap_or_default();
    // The commit's own date, not the day of the build: the same source builds the same text.
    let date = git(&["log", "-1", "--format=%cs"]).unwrap_or_default();
    println!("cargo:rustc-env=VRCAST_BUILD_COMMIT={commit}");
    println!("cargo:rustc-env=VRCAST_BUILD_DATE={date}");

    tauri_build::build()
}
