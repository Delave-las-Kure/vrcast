//! T044–T049 — the library commands.
//!
//! The contract: `contracts/ipc-commands.md`, the "Library" section.
//!
//! The library is centred on media: a person thinks about a work, and the files are its
//! variants. So what goes outside is not a flat directory listing but a list of media with
//! their files nested inside, and, as a group of its own, whatever could not be attributed
//! to anything (FR-015). Hiding the unrecognised will not do: a file that cannot be seen in
//! the application still takes up room on the disk and is still served by its link.

use super::error::{AppError, DetailCode, ErrorCode, Result};
use super::AppState;
use crate::domain::wording::Detail;
use serde::{Deserialize, Serialize};

/// A served file in the form the interface shows it.
///
/// The links are here although `domain::media::MediaFile` has none: that holds facts about
/// the file, while a link is a derived view depending on the profile. Working it out at the
/// boundary is the only way not to hand out a stale address after a domain changes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileView {
    /// The path, relative to the video directory.
    pub path: String,
    pub size_bytes: u64,
    pub duration_s: Option<f64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub bitrate_bps: Option<u64>,
    pub video_codec: Option<String>,
    pub audio_codec: Option<String>,
    /// `moov` at the front of the file. False means a viewer waits for the tail.
    pub faststart_ok: Option<bool>,
    /// False means the file was deleted or renamed outside the application (FR-018).
    pub exists_on_server: bool,
    pub origin_url: String,
    pub cdn_url: Option<String>,
}

/// A medium with all of its files.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaView {
    pub id: String,
    pub title: String,
    pub slug: String,
    pub files: Vec<FileView>,
    /// The quality-ladder descriptions.
    pub ladders: Vec<String>,
    /// How much the medium's files take up in all — what a deletion would free.
    pub total_bytes: u64,
    pub created_at: String,
}

/// Room on the server's disk (FR-017).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiskUsage {
    pub total_bytes: u64,
    pub free_bytes: u64,
    /// How much of what is taken belongs to the serving directory.
    pub used_by_videos_bytes: u64,
}

/// The library, whole.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LibraryView {
    pub server_id: String,
    pub media: Vec<MediaView>,
    /// Files that could not be attributed to any medium (FR-015).
    pub unrecognized: Vec<FileView>,
    /// `None` when the server cannot be reached and there is nowhere to learn the room.
    pub disk: Option<DiskUsage>,
    /// True means the last known state is shown; the server cannot be reached right now.
    ///
    /// An empty screen, or an endless loading spinner, on an unreachable server is the worst
    /// answer possible: a person cannot tell whether they lost their library or their
    /// connection.
    pub stale: bool,
}

impl LibraryView {
    /// How many catalogue entries were accounted for — media files, quality ladders and
    /// the unrecognised together.
    ///
    /// It serves as a completeness check: this number must equal the number of entries in
    /// the serving directory on the server, the housekeeping ones aside. An entry that
    /// landed neither in a medium nor in the "not recognised" group is a lost entry: a
    /// person does not see it, while it takes up room and is served by its link (FR-015).
    ///
    /// A quality ladder counts as one entry rather than a hundred segments: a person thinks
    /// of it as one thing, and showing them every segment would drown the library in
    /// noise.
    pub fn accounted_entries(&self) -> usize {
        self.media
            .iter()
            .map(|m| m.files.len() + m.ladders.len())
            .sum::<usize>()
            + self.unrecognized.len()
    }
}

/// What will be deleted — what a person must see before confirming (FR-014).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeletionImpact {
    pub files: usize,
    pub bytes: u64,
    /// How many connections the web server is serving right now.
    ///
    /// Connections specifically, not viewers of this file: the connection table does not say
    /// what is being downloaded, and there is as yet nothing to attribute them to a
    /// particular medium with. In milestone A the bare fact is enough (FR-019a) — a full
    /// account arrives in Phase 4 along with watching the serving log. Calling this "the
    /// file's viewers" would tell a person something we do not know.
    pub active_connections: usize,
}

/// The thin wrappers for the shell. There is no logic here — only calls into `api`.
pub mod ipc {
    use super::*;
    use crate::domain::links::Links;
    use tauri::State;

    #[tauri::command]
    pub async fn library_list(
        state: State<'_, AppState>,
        server_id: String,
        refresh: Option<bool>,
    ) -> Result<LibraryView> {
        api::library_list(&state, &server_id, refresh.unwrap_or(false)).await
    }

    #[tauri::command]
    pub async fn media_create(
        state: State<'_, AppState>,
        server_id: String,
        title: String,
        slug: Option<String>,
    ) -> Result<String> {
        api::media_create(&state, &server_id, &title, slug.as_deref()).await
    }

    #[tauri::command]
    pub async fn media_rename(
        state: State<'_, AppState>,
        server_id: String,
        media_id: String,
        title: Option<String>,
        slug: Option<String>,
    ) -> Result<()> {
        api::media_rename(
            &state,
            &server_id,
            &media_id,
            title.as_deref(),
            slug.as_deref(),
        )
        .await
    }

    #[tauri::command]
    pub async fn media_delete(
        state: State<'_, AppState>,
        server_id: String,
        media_id: String,
        confirmed: Option<bool>,
    ) -> Result<String> {
        api::media_delete(&state, &server_id, &media_id, confirmed.unwrap_or(false)).await
    }

    #[tauri::command]
    pub async fn file_move(
        state: State<'_, AppState>,
        server_id: String,
        path: String,
        to_media_id: String,
        confirmed: Option<bool>,
    ) -> Result<()> {
        api::file_move(
            &state,
            &server_id,
            &path,
            &to_media_id,
            confirmed.unwrap_or(false),
        )
        .await
    }

    #[tauri::command]
    pub async fn file_delete(
        state: State<'_, AppState>,
        server_id: String,
        path: String,
        confirmed: Option<bool>,
    ) -> Result<()> {
        api::file_delete(&state, &server_id, &path, confirmed.unwrap_or(false)).await
    }

    #[tauri::command]
    pub fn links_for(state: State<'_, AppState>, server_id: String, path: String) -> Result<Links> {
        api::links_for(&state, &server_id, &path)
    }
}

/// Gather what is known about a file, for showing.
fn file_view(
    profile: &crate::domain::server_profile::ServerProfile,
    path: &str,
    size_bytes: u64,
    params: crate::server::probe_moov::FileParams,
    exists_on_server: bool,
) -> FileView {
    let links = crate::domain::links::for_path(&profile.domain, profile.cdn_base.as_deref(), path);
    FileView {
        path: path.to_owned(),
        size_bytes,
        duration_s: params.params.duration_s,
        width: params.params.width,
        height: params.params.height,
        bitrate_bps: params.params.bitrate_bps,
        video_codec: params.params.video_codec,
        audio_codec: params.params.audio_codec,
        faststart_ok: params.faststart_ok,
        exists_on_server,
        origin_url: links.origin,
        cdn_url: links.cdn,
    }
}

pub mod api {
    use super::*;
    use crate::domain::links::Links;
    use crate::domain::manifest::Manifest;
    use crate::domain::media::{self, Media};
    use crate::domain::server_profile::ServerProfile;
    use crate::server::{
        connect, disk, listing, manifest_io, probe_moov, reconcile, SERVICE_ENTRIES,
    };
    use crate::ssh::Connection;
    use crate::store::{library_cache, profiles};

    /// The profile behind an identifier, or a refusal naming it.
    ///
    /// Shared with the other commands that reach a server: two ways of turning an
    /// identifier into a profile would eventually disagree about what happens when there
    /// is none.
    pub fn profile_of(state: &AppState, server_id: &str) -> Result<ServerProfile> {
        profiles::get(&state.db, server_id)?
            .ok_or_else(|| crate::commands::servers::no_such_server(server_id))
    }

    /// A server's library.
    ///
    /// Without `refresh` the cache is handed back — instantly — while the refresh follows
    /// and arrives as an event. There is no point waiting for the server's answer to show a
    /// list that is already known: over a slow link that is seconds of empty screen.
    pub async fn library_list(
        state: &AppState,
        server_id: &str,
        refresh: bool,
    ) -> Result<LibraryView> {
        let profile = profile_of(state, server_id)?;

        if !refresh {
            if let Some(cached) = library_cache::load(&state.db, server_id)? {
                // The refresh goes its own way: a person already sees the list, and any
                // divergence from the server will arrive as an event and correct it.
                spawn_background_refresh(state.clone(), profile.clone());
                return Ok(cached);
            }
        }

        match build_from_server(state, &profile).await {
            Ok(view) => {
                library_cache::save(&state.db, server_id, &view)?;
                Ok(view)
            }
            Err(e) => {
                // The server cannot be reached. Showing the last known state with a mark
                // on it beats an empty screen: an empty one is indistinguishable from
                // "the library is gone".
                match library_cache::load(&state.db, server_id)? {
                    Some(mut cached) => {
                        tracing::warn!(server = server_id, error = %e, "library taken from the cache");
                        cached.stale = true;
                        Ok(cached)
                    }
                    None => Err(e),
                }
            }
        }
    }

    /// Refresh the cache aside from the answer, and report the change.
    fn spawn_background_refresh(state: AppState, profile: ServerProfile) {
        let server_id = profile.id.clone();
        tokio::spawn(async move {
            match build_from_server(&state, &profile).await {
                Ok(view) => {
                    if library_cache::save(&state.db, &server_id, &view).is_ok() {
                        state.notify_library_changed(&server_id);
                    }
                }
                Err(e) => {
                    tracing::debug!(server = %server_id, error = %e, "the background library refresh failed")
                }
            }
        });
    }

    /// Read the whole library from the server.
    async fn build_from_server(state: &AppState, profile: &ServerProfile) -> Result<LibraryView> {
        let conn = connect(state.secrets.as_ref(), profile).await?;
        let dir = &profile.video_dir;

        let manifest = manifest_io::read(&conn, dir).await?;
        let entries = listing::list(&conn, dir).await?;
        let matched = reconcile::reconcile(&manifest, &entries);

        // Room on the disk is no reason to refuse the library: even when it cannot be
        // learned, the list is useful all the same.
        let disk_usage = match disk::usage(&conn, dir).await {
            Ok(u) => Some(u),
            Err(e) => {
                tracing::warn!(error = %e, "the room on the server's disk was not read");
                None
            }
        };

        let mut media_views = Vec::with_capacity(manifest.media.len());
        for (media, files) in manifest.media.iter().zip(matched.media_files.iter()) {
            let mut views = Vec::with_capacity(files.files.len());
            let mut total = 0u64;
            for f in &files.files {
                total += f.size_bytes;
                let params = probed(state, &conn, profile, &f.path, f.size_bytes, f.exists).await;
                views.push(file_view(profile, &f.path, f.size_bytes, params, f.exists));
            }
            // A quality ladder counts towards the medium's size: deleting frees it too.
            total += files.ladders.iter().map(|l| l.size_bytes).sum::<u64>();

            media_views.push(MediaView {
                id: media.id.clone(),
                title: media.title.clone(),
                slug: media.slug.clone(),
                files: views,
                ladders: files.ladders.iter().map(|l| l.path.clone()).collect(),
                total_bytes: total,
                created_at: media.created_at.clone(),
            });
        }

        let mut unrecognized = Vec::with_capacity(matched.unrecognized.len());
        for entry in &matched.unrecognized {
            // We do not look inside a directory: a directory has no header, and going
            // through its contents for who knows what is needless network round trips.
            let params = if entry.is_dir {
                probe_moov::FileParams::default()
            } else {
                probed(state, &conn, profile, &entry.name, entry.size_bytes, true).await
            };
            unrecognized.push(file_view(
                profile,
                &entry.name,
                entry.size_bytes,
                params,
                true,
            ));
        }

        conn.close().await;

        Ok(LibraryView {
            server_id: profile.id.clone(),
            media: media_views,
            unrecognized,
            disk: disk_usage,
            stale: false,
        })
    }

    /// A file's parameters from its header. A failed parse is no reason to lose the file.
    async fn probed(
        state: &AppState,
        conn: &Connection,
        profile: &ServerProfile,
        path: &str,
        size_bytes: u64,
        exists: bool,
    ) -> probe_moov::FileParams {
        if !exists || size_bytes == 0 || path.contains('/') {
            return probe_moov::FileParams::default();
        }
        probe_moov::params_for(
            conn,
            &state.db,
            &profile.id,
            &profile.video_dir,
            path,
            size_bytes,
        )
        .await
        .unwrap_or_else(|e| {
            tracing::debug!(file = path, error = %e, "the file's header was not read");
            probe_moov::FileParams::default()
        })
    }

    /// Create a medium. The `slug` is unique within a server; an empty one is made from
    /// the title.
    pub async fn media_create(
        state: &AppState,
        server_id: &str,
        title: &str,
        slug: Option<&str>,
    ) -> Result<String> {
        let profile = profile_of(state, server_id)?;

        let title = title.trim();
        if title.is_empty() {
            return Err(AppError::new(ErrorCode::InvalidInput).detail(DetailCode::MediaTitleEmpty));
        }

        let slug = match slug.map(str::trim).filter(|s| !s.is_empty()) {
            Some(s) => s.to_owned(),
            None => media::slugify(title).ok_or_else(|| {
                AppError::new(ErrorCode::InvalidInput).detail(DetailCode::SlugUnmakeable)
            })?,
        };
        media::validate_slug(&slug)
            .map_err(|e| AppError::new(ErrorCode::InvalidInput).with_detail(e.detail()))?;

        let conn = connect(state.secrets.as_ref(), &profile).await?;
        let manifest = manifest_io::read(&conn, &profile.video_dir).await?;

        if !manifest.slug_available(&slug, None) {
            conn.close().await;
            return Err(AppError::new(ErrorCode::SlugTaken).with_cause(&slug));
        }

        let id = format!("m_{}", uuid::Uuid::new_v4().simple());
        let mut next = manifest.prepared_for_write();
        next.media.push(Media::new(
            &id,
            title,
            &slug,
            crate::store::db::now_rfc3339(),
        ));

        manifest_io::write(&conn, &profile.video_dir, &next, manifest.generation).await?;
        conn.close().await;

        invalidate(state, server_id);
        Ok(id)
    }

    /// Rename a medium.
    ///
    /// Changing the short name renames the files and **breaks the old links**: the
    /// interface must warn about that before calling.
    pub async fn media_rename(
        state: &AppState,
        server_id: &str,
        media_id: &str,
        title: Option<&str>,
        slug: Option<&str>,
    ) -> Result<()> {
        let profile = profile_of(state, server_id)?;

        let new_title = title.map(str::trim).filter(|t| !t.is_empty());
        let new_slug = slug.map(str::trim).filter(|s| !s.is_empty());
        if new_title.is_none() && new_slug.is_none() {
            return Err(
                AppError::new(ErrorCode::InvalidInput).detail(DetailCode::MediaNothingToChange)
            );
        }
        if let Some(s) = new_slug {
            media::validate_slug(s)
                .map_err(|e| AppError::new(ErrorCode::InvalidInput).with_detail(e.detail()))?;
        }

        let conn = connect(state.secrets.as_ref(), &profile).await?;
        let manifest = manifest_io::read(&conn, &profile.video_dir).await?;

        let Some(index) = manifest.media.iter().position(|m| m.id == media_id) else {
            conn.close().await;
            return Err(no_such_media(media_id));
        };
        if let Some(s) = new_slug {
            if !manifest.slug_available(s, Some(media_id)) {
                conn.close().await;
                return Err(AppError::new(ErrorCode::SlugTaken).with_cause(s));
            }
        }

        let mut next = manifest.prepared_for_write();
        let media = &mut next.media[index];
        if let Some(t) = new_title {
            media.title = t.to_owned();
        }

        if let Some(s) = new_slug {
            let old = media.slug.clone();
            if s != old {
                rename_entries(&conn, &profile.video_dir, media, &old, s).await?;
                media.slug = s.to_owned();
            }
        }

        manifest_io::write(&conn, &profile.video_dir, &next, manifest.generation).await?;
        conn.close().await;

        invalidate(state, server_id);
        Ok(())
    }

    /// Rename the catalogue entries to follow a short name.
    ///
    /// The renaming happens **before** the catalogue is written: should it fail, the
    /// catalogue stays as it was and still matches what is on the server. The other order
    /// would give a catalogue pointing at files that do not exist.
    async fn rename_entries(
        conn: &Connection,
        video_dir: &str,
        media: &mut Media,
        old_slug: &str,
        new_slug: &str,
    ) -> Result<()> {
        use crate::server::{join_remote, shell_quote};

        // The top-level entries that have to be renamed: the name equals the old short
        // name entirely, or begins with it.
        let mut renames: Vec<(String, String)> = Vec::new();
        let mut rename_top = |path: &str| -> String {
            let (top, rest) = match path.split_once('/') {
                Some((t, r)) => (t, Some(r)),
                None => (path, None),
            };
            let new_top = if top == old_slug {
                new_slug.to_owned()
            } else if let Some(tail) = top.strip_prefix(old_slug) {
                format!("{new_slug}{tail}")
            } else {
                // The file does not follow the naming convention — it must not be
                // touched: a person may have attributed something of their own naming to
                // the medium.
                return path.to_owned();
            };
            if top != new_top && !renames.iter().any(|(o, _)| o == top) {
                renames.push((top.to_owned(), new_top.clone()));
            }
            match rest {
                Some(r) => format!("{new_top}/{r}"),
                None => new_top,
            }
        };

        let new_files: Vec<String> = media.files.iter().map(|p| rename_top(p)).collect();
        let new_ladders: Vec<String> = media.ladders.iter().map(|p| rename_top(p)).collect();

        for (old, new) in &renames {
            let out = conn
                .exec(&format!(
                    "mv -n -- {} {}",
                    shell_quote(&join_remote(video_dir, old)),
                    shell_quote(&join_remote(video_dir, new))
                ))
                .await?;
            if !out.ok() {
                return Err(AppError::new(ErrorCode::Internal)
                    .with_detail(
                        Detail::new(DetailCode::RenameFailed)
                            .with("old", old.to_string())
                            .with("new", new.to_string()),
                    )
                    .with_cause(out.stderr.trim()));
            }
        }

        media.files = new_files;
        media.ladders = new_ladders;
        Ok(())
    }

    /// Delete a medium along with its files.
    ///
    /// Without `confirmed` it comes back as a **refusal** naming the number of files and
    /// the volume: there is nothing to confirm blind (FR-014).
    ///
    /// Deleting is not a task, although the contract names a `task_id`: removing files is an
    /// operation counted in files rather than in bytes, and takes fractions of a second even
    /// over tens of gigabytes. The deleted medium's identifier is returned.
    pub async fn media_delete(
        state: &AppState,
        server_id: &str,
        media_id: &str,
        confirmed: bool,
    ) -> Result<String> {
        let profile = profile_of(state, server_id)?;
        let conn = connect(state.secrets.as_ref(), &profile).await?;
        let manifest = manifest_io::read(&conn, &profile.video_dir).await?;

        let Some(index) = manifest.media.iter().position(|m| m.id == media_id) else {
            conn.close().await;
            return Err(no_such_media(media_id));
        };

        if !confirmed {
            let impact = impact_of(&conn, &profile, &manifest, index).await;
            conn.close().await;
            return Err(confirmation_needed(&manifest.media[index].title, &impact));
        }

        let media = manifest.media[index].clone();
        remove_entries(&conn, &profile.video_dir, media.all_paths()).await?;

        let mut next = manifest.prepared_for_write();
        next.media.remove(index);
        manifest_io::write(&conn, &profile.video_dir, &next, manifest.generation).await?;
        conn.close().await;

        invalidate(state, server_id);
        tracing::info!(
            media = media_id,
            "the medium was deleted along with its files"
        );
        Ok(media_id.to_owned())
    }

    /// Move a file into another medium.
    ///
    /// The file stays where it is — only which medium it belongs to changes. Renaming it to
    /// follow the new short name will not do: that would break working links, which nobody
    /// asked for.
    pub async fn file_move(
        state: &AppState,
        server_id: &str,
        path: &str,
        to_media_id: &str,
        _confirmed: bool,
    ) -> Result<()> {
        let profile = profile_of(state, server_id)?;
        let conn = connect(state.secrets.as_ref(), &profile).await?;
        let manifest = manifest_io::read(&conn, &profile.video_dir).await?;

        if manifest.find_by_id(to_media_id).is_none() {
            conn.close().await;
            return Err(no_such_media(to_media_id));
        }

        let entries = listing::list(&conn, &profile.video_dir).await?;
        let top = path.split('/').next().unwrap_or(path);
        if !entries.iter().any(|e| e.name == top) {
            conn.close().await;
            return Err(AppError::new(ErrorCode::FileMissingOnServer).with_cause(path));
        }

        let mut next = manifest.prepared_for_write();
        for m in &mut next.media {
            m.files.retain(|p| p != path);
            m.ladders.retain(|p| p != path);
        }
        if let Some(target) = next.media.iter_mut().find(|m| m.id == to_media_id) {
            if path.ends_with(".m3u8") {
                target.ladders.push(path.to_owned());
            } else {
                target.files.push(path.to_owned());
            }
        }

        manifest_io::write(&conn, &profile.video_dir, &next, manifest.generation).await?;
        conn.close().await;

        invalidate(state, server_id);
        Ok(())
    }

    /// Delete one file.
    ///
    /// Without `confirmed` — a refusal naming the volume that would be freed (FR-014).
    pub async fn file_delete(
        state: &AppState,
        server_id: &str,
        path: &str,
        confirmed: bool,
    ) -> Result<()> {
        let profile = profile_of(state, server_id)?;
        let conn = connect(state.secrets.as_ref(), &profile).await?;

        let entries = listing::list(&conn, &profile.video_dir).await?;
        let top = path.split('/').next().unwrap_or(path);
        let Some(entry) = entries.iter().find(|e| e.name == top) else {
            conn.close().await;
            return Err(AppError::new(ErrorCode::FileMissingOnServer).with_cause(path));
        };
        if SERVICE_ENTRIES.contains(&top) {
            conn.close().await;
            return Err(
                AppError::new(ErrorCode::InvalidInput).detail(DetailCode::MediaIsServiceEntry)
            );
        }

        if !confirmed {
            let impact = DeletionImpact {
                files: 1,
                bytes: entry.size_bytes,
                active_connections: active_connections(&conn).await,
            };
            conn.close().await;
            return Err(confirmation_needed(path, &impact));
        }

        remove_entries(&conn, &profile.video_dir, std::iter::once(&path.to_owned())).await?;

        // It leaves the catalogue in the same act: the file is gone, and a reference to it
        // in the catalogue would turn into a file forever missing.
        let manifest = manifest_io::read(&conn, &profile.video_dir).await?;
        if manifest.all_claimed_paths().contains(&path) {
            let mut next = manifest.prepared_for_write();
            for m in &mut next.media {
                m.files.retain(|p| p != path);
                m.ladders.retain(|p| p != path);
            }
            manifest_io::write(&conn, &profile.video_dir, &next, manifest.generation).await?;
        }
        conn.close().await;

        invalidate(state, server_id);
        Ok(())
    }

    /// The viewers' links to a file (FR-016).
    pub fn links_for(state: &AppState, server_id: &str, path: &str) -> Result<Links> {
        let profile = profile_of(state, server_id)?;
        Ok(crate::domain::links::for_path(
            &profile.domain,
            profile.cdn_base.as_deref(),
            path,
        ))
    }

    // ---------- helpers ----------

    fn no_such_media(id: &str) -> AppError {
        AppError::new(ErrorCode::InvalidInput)
            .detail(DetailCode::MediaNotFound)
            .with_cause(id)
    }

    /// A refusal that names the consequences. Without the numbers there would be nothing
    /// to confirm.
    fn confirmation_needed(what: &str, impact: &DeletionImpact) -> AppError {
        let mut error = AppError::new(ErrorCode::ConfirmationRequired).with_detail(
            Detail::new(DetailCode::ConfirmDelete)
                .with("what", what.to_string())
                .with("files", impact.files)
                .with("bytes", impact.bytes),
        );
        // A second thing to say, not a longer first one: whether anyone is watching
        // right now is a separate fact, and it is worded the same wherever it comes up.
        if impact.active_connections > 0 {
            error = error.with_detail(
                Detail::new(DetailCode::ViewersActiveDelete)
                    .with("connections", impact.active_connections),
            );
        }
        error.with_cause(format!(
            "files={}, bytes={}, connections={}",
            impact.files, impact.bytes, impact.active_connections
        ))
    }

    async fn impact_of(
        conn: &Connection,
        profile: &ServerProfile,
        manifest: &Manifest,
        index: usize,
    ) -> DeletionImpact {
        let media = &manifest.media[index];
        let entries = listing::list(conn, &profile.video_dir)
            .await
            .unwrap_or_default();
        let matched = reconcile::reconcile(manifest, &entries);

        let files = matched.media_files.get(index);
        let bytes = files.map_or(0, |f| {
            f.files.iter().map(|x| x.size_bytes).sum::<u64>()
                + f.ladders.iter().map(|x| x.size_bytes).sum::<u64>()
        });

        DeletionImpact {
            files: media.files.len() + media.ladders.len(),
            bytes,
            active_connections: active_connections(conn).await,
        }
    }

    /// How many connections the web server is serving right now (FR-019a).
    ///
    /// It lives in `server::active_use`: the same thing is needed before an upload (FR-037),
    /// and two copies of one count would diverge at the first edit.
    async fn active_connections(conn: &Connection) -> usize {
        crate::server::active_use::serving_connections(conn).await
    }

    /// Delete catalogue entries — both files and quality-ladder directories.
    async fn remove_entries<'a>(
        conn: &Connection,
        video_dir: &str,
        paths: impl Iterator<Item = &'a String>,
    ) -> Result<()> {
        use crate::server::{join_remote, shell_quote};

        // Top-level entries are what gets deleted: a quality ladder is a whole directory,
        // and removing it a segment at a time would leave half of it behind.
        let mut tops: Vec<String> = Vec::new();
        for path in paths {
            let top = path.split('/').next().unwrap_or(path).to_owned();
            if SERVICE_ENTRIES.contains(&top.as_str()) {
                continue;
            }
            if !tops.contains(&top) {
                tops.push(top);
            }
        }
        if tops.is_empty() {
            return Ok(());
        }

        let args = tops
            .iter()
            .map(|t| shell_quote(&join_remote(video_dir, t)))
            .collect::<Vec<_>>()
            .join(" ");
        let out = conn.exec(&format!("rm -rf -- {args}")).await?;
        if !out.ok() {
            return Err(AppError::new(ErrorCode::Internal)
                .detail(DetailCode::DeleteFilesFailed)
                .with_cause(out.stderr.trim()));
        }
        Ok(())
    }

    /// Forget the cache: after a change it certainly no longer matches the server.
    fn invalidate(state: &AppState, server_id: &str) {
        if let Err(e) = library_cache::forget(&state.db, server_id) {
            tracing::warn!(server = server_id, error = %e, "the library cache was not cleared");
        }
        state.notify_library_changed(server_id);
    }
}
