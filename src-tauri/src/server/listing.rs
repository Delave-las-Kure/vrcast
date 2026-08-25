//! T044 — what is in the serving directory.
//!
//! The **top level** is read: a file is an entry, and so is a directory (usually a
//! quality ladder). There is no reason to descend into a ladder: a person thinks of it
//! as one thing, and showing them every segment would drown the library in noise.
//!
//! The listing is not filtered: it honestly gives back everything visible on the
//! server. What of it to show is decided higher up — otherwise the filter would have
//! to be remembered in every place the listing is used.

use crate::ssh::{Connection, Result, SshError};

/// One entry of the serving directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The name, relative to the serving directory.
    pub name: String,
    /// The size; for a directory, the total size of what is inside it.
    pub size_bytes: u64,
    pub is_dir: bool,
}

/// Read the contents of the serving directory.
pub async fn list(conn: &Connection, video_dir: &str) -> Result<Vec<Entry>> {
    let sftp = conn.sftp().await?;

    let entries = sftp
        .read_dir(video_dir)
        .await
        .map_err(|e| SshError::sftp(crate::store::redact::safe_display(&e)))?;

    let mut out = Vec::new();
    for e in entries {
        let name = e.file_name();
        if name == "." || name == ".." {
            continue;
        }
        let meta = e.metadata();
        out.push(Entry {
            name,
            size_bytes: meta.size.unwrap_or(0),
            is_dir: meta.is_dir(),
        });
    }

    // Directories arrived with the size of the directory entry itself, not of what is
    // inside. They are totalled with one command for all of them at once: walking each
    // quality ladder separately is dozens of calls to the server where one will do.
    let dirs: Vec<&str> = out
        .iter()
        .filter(|e| e.is_dir)
        .map(|e| e.name.as_str())
        .collect();
    if !dirs.is_empty() {
        let sizes = directory_sizes(conn, video_dir, &dirs).await?;
        for entry in out.iter_mut().filter(|e| e.is_dir) {
            if let Some(size) = sizes.get(entry.name.as_str()) {
                entry.size_bytes = *size;
            }
        }
    }

    // A stable order: a person sees this listing, and it must not jump about from one
    // call to the next.
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

/// The sizes of the listed directories, in one command.
async fn directory_sizes(
    conn: &Connection,
    video_dir: &str,
    dirs: &[&str],
) -> Result<std::collections::HashMap<String, u64>> {
    let args = dirs
        .iter()
        .map(|d| super::shell_quote(&super::join_remote(video_dir, d)))
        .collect::<Vec<_>>()
        .join(" ");
    let out = conn.exec(&format!("du -sb -- {args} 2>/dev/null")).await?;

    let mut map = std::collections::HashMap::new();
    for line in out.stdout.lines() {
        let Some((size, path)) = line.split_once('\t') else {
            continue;
        };
        let Ok(size) = size.trim().parse::<u64>() else {
            continue;
        };
        // The directory name is the last segment of the path. Comparing against the
        // original name is sturdier than relying on the order of the output lines.
        let name = path.trim().rsplit('/').next().unwrap_or("").to_owned();
        if !name.is_empty() {
            map.insert(name, size);
        }
    }
    Ok(map)
}
