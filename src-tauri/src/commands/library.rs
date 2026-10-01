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
use crate::domain::grouping::Suggestion;
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

/// A quality set in the form the interface shows it (FR-012, T529).
///
/// **Not a bare path.** A file served directly carries its resolution, bitrate and length
/// (`FileView`); a set is the same kind of thing — a viewer plays it the same way — and
/// showing it as a string that merely names a directory answered none of the same
/// questions. The particulars come from what the set itself already records: the master
/// playlist's own numbers for the heaviest rung (`server::ladder_probe`), never a guess and
/// never the ladder that was *asked* for — an encoder does not deliver exactly what it was
/// told (see `domain::hls_master`'s own doc comment).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LadderSetView {
    /// The description's path, relative to the video directory: `{slug}/master.m3u8`.
    pub path: String,
    /// The whole directory's size — every rung together, which is what a deletion frees.
    pub size_bytes: u64,
    /// The heaviest rung's own numbers. `None` when the master could not be read or
    /// parsed — an older set, or one this application did not build.
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub bitrate_bps: Option<u64>,
    pub duration_s: Option<f64>,
    /// False means the directory was deleted or renamed outside the application (FR-018).
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
    /// The quality sets built for this medium.
    pub ladders: Vec<LadderSetView>,
    /// The prepared rung files of the medium's set (T678): `{slug}_{N}.mp4` beside `{slug}/`.
    /// Not files to hand out — the set is — but they take room and go when the medium goes,
    /// so they are shown with it rather than as «not recognised».
    #[serde(default)]
    pub set_files: Vec<FileView>,
    /// How much the medium's files take up in all — what a deletion would free.
    pub total_bytes: u64,
    pub created_at: String,
    /// A video on the «Video» screen building this medium's set (T677): `building`, or
    /// `stopped` on a problem or by a person. `null` when none is. Not read from the server —
    /// filled in from this machine's videos every time the library is handed out, so a set
    /// being rebuilt after «Replace» reads as on its way rather than as missing.
    #[serde(default)]
    pub set_work: Option<crate::domain::video::SetWork>,
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
            .map(|m| m.files.len() + m.ladders.len() + m.set_files.len())
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
    /// The set's prepared rung files among `files` (T678), by name — said separately, so a
    /// person sees what they are.
    #[serde(default)]
    pub set_files: Vec<String>,
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
        cached_only: Option<bool>,
    ) -> Result<LibraryView> {
        // `cached_only` (T651) — what a screen reads on `library:changed`: the answer, not
        // another question. `refresh` wins over it: asking for the server outright is the
        // stronger request.
        if refresh.unwrap_or(false) || !cached_only.unwrap_or(false) {
            api::library_list(&state, &server_id, refresh.unwrap_or(false)).await
        } else {
            api::library_list_known(&state, &server_id).await
        }
    }

    #[tauri::command]
    pub async fn library_suggest_groups(
        state: State<'_, AppState>,
        server_id: String,
    ) -> Result<Suggestion> {
        api::library_suggest_groups(&state, &server_id).await
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
        confirmed: Option<bool>,
    ) -> Result<()> {
        api::media_rename(
            &state,
            &server_id,
            &media_id,
            title.as_deref(),
            slug.as_deref(),
            confirmed.unwrap_or(false),
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

/// Gather what is known about a quality set, for showing (T529).
///
/// `path` is `{slug}/master.m3u8` as the catalogue records it; the slug is the top-level
/// directory the whole set lives under, and that is what `ladder_probe::top_rung` is asked
/// about. Reading fails silently into blanks rather than losing the set from view: a build
/// from before `.facts` existed, or one this application did not make, still deserves to be
/// seen and deleted like any other entry.
async fn ladder_view(
    profile: &crate::domain::server_profile::ServerProfile,
    conn: &crate::ssh::Connection,
    path: &str,
    size_bytes: u64,
    exists_on_server: bool,
) -> LadderSetView {
    let links = crate::domain::links::for_path(&profile.domain, profile.cdn_base.as_deref(), path);
    let top = if exists_on_server {
        let slug = path.split('/').next().unwrap_or(path);
        crate::server::ladder_probe::top_rung(conn, &profile.video_dir, slug).await
    } else {
        None
    };
    LadderSetView {
        path: path.to_owned(),
        size_bytes,
        width: top.as_ref().map(|t| t.width),
        height: top.as_ref().map(|t| t.height),
        bitrate_bps: top.as_ref().map(|t| t.bitrate_bps),
        duration_s: top.as_ref().and_then(|t| t.duration_s),
        exists_on_server,
        origin_url: links.origin,
        cdn_url: links.cdn,
    }
}

/// The refreshes of a library under way, one per server (T651).
///
/// **Why a refresh is shared rather than simply started.** Every `library_list` without
/// `refresh` used to start a whole new read of the server behind the cache it handed back —
/// the connection, the catalogue, the listing, the disk and a header probe per file — and
/// every screen opening the library, the viewers' watch and every `library:changed` asked
/// again. Two screens open at once read the same server twice at the same moment for the
/// same answer. Here a refresh already under way is joined instead, and a new one starts
/// only when none is.
///
/// **Why not joined across a change.** A catalogue change (`AppState::invalidate_library`)
/// makes a refresh started before it useless: it may have read the catalogue before the
/// change was written. [`invalidated`] marks that; the old refresh is left to finish on its
/// own for whoever already waits on it, but nobody new joins it and its answer is not kept
/// — otherwise a person who renamed something could be shown, and have cached, the old name.
///
/// **Why `library:changed` only on a difference.** The event means "the library changed,
/// read it again", and a screen does exactly that. A refresh that found everything as the
/// cache already had it has nothing to say; sending the event anyway is what closed the loop
/// cache → refresh → event → cache → refresh the audit found (QA-24A №2).
///
/// Public for the contract tests, which are a separate crate: `run` is how they check that
/// two refreshes of one server read it once, and [`settle`] that an unchanged answer sends
/// no event.
pub mod refreshes {
    use super::{AppError, ErrorCode, LibraryView, Result};
    use crate::commands::AppState;
    use crate::store::db::Db;
    use crate::store::library_cache;
    use futures::future::{BoxFuture, FutureExt, Shared};
    use std::collections::HashMap;
    use std::sync::{LazyLock, Mutex};

    /// A refresh that anyone may wait on; all of them get the same answer.
    pub type Refresh = Shared<BoxFuture<'static, Result<LibraryView>>>;

    #[derive(Default)]
    struct Entry {
        /// Bumped by every catalogue change — see [`invalidated`].
        epoch: u64,
        /// How many refreshes of this server are still running, joinable or not. The entry
        /// is dropped at zero: once nothing runs, its epoch no longer compares with anything.
        running: u32,
        next_id: u64,
        /// The refresh a newcomer joins: its id, the epoch it started in, and itself.
        joinable: Option<(u64, u64, Refresh)>,
    }

    /// Keyed by the database as well as the server: tests build many states side by side,
    /// with server identifiers of their own that may coincide. A running refresh holds a
    /// clone of its `AppState` and with it the database, so an address cannot be reused by
    /// another database while an entry still names it.
    type Key = (usize, String);

    static RUNNING: LazyLock<Mutex<HashMap<Key, Entry>>> = LazyLock::new(Default::default);

    fn key(db: &Db, server_id: &str) -> Key {
        (db as *const Db as usize, server_id.to_owned())
    }

    fn registry() -> std::sync::MutexGuard<'static, HashMap<Key, Entry>> {
        // A panic while holding this lock leaves nothing half-written that matters: the
        // map only says what is running.
        RUNNING.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// The refresh of this server's library: the one under way when there is one begun
    /// since the last catalogue change, otherwise a new one.
    ///
    /// The work runs on its own task, so dropping what is returned does not stop it — that
    /// is how a refresh "follows" the cached answer without anyone waiting for it.
    pub fn run<F>(state: &AppState, server_id: &str, build: F) -> Refresh
    where
        F: FnOnce() -> BoxFuture<'static, Result<LibraryView>> + Send + 'static,
    {
        let key = key(&state.db, server_id);
        let mut map = registry();
        let entry = map.entry(key.clone()).or_default();
        if let Some((_, epoch, refresh)) = &entry.joinable {
            if *epoch == entry.epoch {
                return refresh.clone();
            }
        }

        let (id, epoch) = (entry.next_id, entry.epoch);
        entry.next_id += 1;
        entry.running += 1;

        let finish = Finish {
            key,
            id,
            state: state.clone(),
        };
        // Recorded as joinable before the work is spawned — and the registry let go of
        // before spawning, since a task dropped on the spot (a runtime shutting down) runs
        // `Finish` at once, which takes the registry too. The answer travels through a
        // channel so that it can be recorded before the task exists.
        let (tx, rx) = tokio::sync::oneshot::channel::<Result<LibraryView>>();
        let refresh: Refresh = async move {
            rx.await.unwrap_or_else(|_| {
                tracing::error!("the library refresh stopped abnormally");
                Err(AppError::new(ErrorCode::Internal))
            })
        }
        .boxed()
        .shared();
        entry.joinable = Some((id, epoch, refresh.clone()));
        drop(map);

        let server = server_id.to_owned();
        tokio::spawn(async move {
            let outcome = build().await;
            let map = registry();
            let fresh = map.get(&finish.key).is_some_and(|e| e.epoch == epoch);
            match &outcome {
                Ok(view) if fresh => {
                    // Under the lock, so that a catalogue change cannot slip between the
                    // epoch check and the write and be overwritten by what preceded it.
                    if let Err(e) = settle(&finish.state, &server, view) {
                        tracing::warn!(server = %server, error = %e, "the refreshed library was not cached");
                    }
                }
                Ok(_) => {
                    tracing::debug!(server = %server, "a library refresh overtaken by a change was not kept")
                }
                Err(e) => {
                    tracing::debug!(server = %server, error = %e, "the library refresh failed")
                }
            }
            drop(map);
            // No longer joinable before the answer goes out: whoever asks after this
            // point gets a read of their own, not an answer that is already history.
            drop(finish);
            let _ = tx.send(outcome);
        });
        refresh
    }

    /// The bookkeeping at the end of a refresh, however it ends — finished, panicked, or
    /// dropped with its runtime. Otherwise the server's entry would stay "running" and every
    /// later refresh would join a dead one.
    ///
    /// Holds the refresh's `AppState` and lets it go only **after** the entry is cleared:
    /// the key is the database's address, and that address must not be free for another
    /// database while an entry still names it.
    struct Finish {
        key: Key,
        id: u64,
        state: AppState,
    }

    impl Drop for Finish {
        fn drop(&mut self) {
            let mut map = registry();
            if let Some(e) = map.get_mut(&self.key) {
                if e.joinable.as_ref().is_some_and(|(j, _, _)| *j == self.id) {
                    e.joinable = None;
                }
                e.running = e.running.saturating_sub(1);
                if e.running == 0 {
                    map.remove(&self.key);
                }
            }
        }
    }

    /// A catalogue change was written: a refresh begun before it is not to be joined or
    /// kept. Called by `invalidate_library_parts` before it forgets the cache.
    pub fn invalidated(db: &Db, server_id: &str) {
        if let Some(e) = registry().get_mut(&key(db, server_id)) {
            e.epoch += 1;
        }
    }

    /// Keep a freshly read library, and say so only if it differs from what was kept.
    ///
    /// Returns whether it differed. No cache at all counts as a difference: whatever
    /// forgot it expects the library to be read again.
    pub fn settle(
        state: &AppState,
        server_id: &str,
        view: &LibraryView,
    ) -> std::result::Result<bool, crate::store::db::DbError> {
        let changed = library_cache::load(&state.db, server_id)?.as_ref() != Some(view);
        library_cache::save(&state.db, server_id, view)?;
        if changed {
            state.notify_library_changed(server_id);
        }
        Ok(changed)
    }
}

pub mod api {
    use super::*;
    use crate::domain::links::Links;
    use crate::domain::manifest::Manifest;
    use crate::domain::media::{self, Media};
    use crate::domain::server_profile::ServerProfile;
    use crate::server::gate::{self, Intent};
    use crate::server::{
        disk, listing, manifest_io, probe_moov, reconcile, set_files, SERVICE_ENTRIES,
    };
    use crate::ssh::connection::BRIEF_CHANNELS;
    use crate::ssh::Connection;
    use crate::store::{library_cache, profiles};
    use futures::stream::{self, StreamExt};
    use futures::FutureExt;

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
        let mode = if refresh {
            Read::Server
        } else {
            Read::CacheThenRefresh
        };
        read(state, server_id, mode).await
    }

    /// A server's library as it is already known — **without** asking the server for a
    /// refresh (T651).
    ///
    /// This is what a screen reads on `library:changed`. That event *is* the end of a
    /// refresh; answering it with `library_list(refresh=false)` asked for another refresh,
    /// whose end sent another event — an endless round of reading the whole server while a
    /// person merely had the library open. Reading the answer and asking the question are
    /// two different things, and this is only the first.
    ///
    /// Only when there is no cache at all (a change just forgot it) is the server read —
    /// there is nothing else to show — and that read joins any already under way.
    pub async fn library_list_known(state: &AppState, server_id: &str) -> Result<LibraryView> {
        read(state, server_id, Read::Cache).await
    }

    /// How a library is to be read.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Read {
        /// The cache when there is one; the server otherwise.
        Cache,
        /// The cache when there is one, with a refresh started behind it.
        CacheThenRefresh,
        /// The server, now.
        Server,
    }

    async fn read(state: &AppState, server_id: &str, mode: Read) -> Result<LibraryView> {
        read_from(state, server_id, mode)
            .await
            .map(|view| with_set_work(state, view))
    }

    /// Mark each medium whose set a video on this machine is building (T677).
    ///
    /// Applied to whatever is handed out — from the server, the cache, or the cache marked
    /// stale — and never kept in the cache: it changes with the videos, not with the server.
    /// The most telling video wins when there are several (one going over one stopped).
    pub fn with_set_work(state: &AppState, mut view: LibraryView) -> LibraryView {
        use crate::domain::video::{set_work_of, SetWork, SetWorkState};
        let videos = crate::store::videos::list(&state.db).unwrap_or_else(|e| {
            tracing::warn!(error = %e, "the videos were not read for the library");
            Vec::new()
        });
        for media in &mut view.media {
            media.set_work = videos
                .iter()
                .filter(|v| {
                    v.server_id == view.server_id && v.media_id.as_deref() == Some(&media.id)
                })
                .filter_map(|v| {
                    set_work_of(v.state, v.stage, v.start_requested).map(|s| SetWork {
                        state: s,
                        video_id: v.id.clone(),
                    })
                })
                .min_by_key(|w| w.state != SetWorkState::Building);
        }
        view
    }

    async fn read_from(state: &AppState, server_id: &str, mode: Read) -> Result<LibraryView> {
        let profile = profile_of(state, server_id)?;

        if mode != Read::Server {
            if let Some(cached) = library_cache::load(&state.db, server_id)? {
                if mode == Read::CacheThenRefresh {
                    // The refresh goes its own way: a person already sees the list, and a
                    // divergence from the server will arrive as an event and correct it.
                    // Nothing waits for it — it is already running on its own.
                    drop(refresh(state, &profile));
                }
                return Ok(cached);
            }
        }

        match refresh(state, &profile).await {
            Ok(view) => Ok(view),
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

    /// The one refresh of this server's library — joining the one already under way, if
    /// any (T651).
    fn refresh(state: &AppState, profile: &ServerProfile) -> super::refreshes::Refresh {
        let (st, p) = (state.clone(), profile.clone());
        super::refreshes::run(state, &profile.id, move || {
            async move { build_from_server(&st, &p).await }.boxed()
        })
    }

    /// Read the whole library from the server.
    async fn build_from_server(state: &AppState, profile: &ServerProfile) -> Result<LibraryView> {
        // Looking. Allowed even on somebody else's machine — looking is how a person finds
        // out that it *is* somebody else's.
        let conn = gate::open(state.secrets.as_ref(), profile, Intent::Read)
            .await?
            .conn;
        let dir = &profile.video_dir;

        let manifest = manifest_io::read(&conn, dir).await?;
        let entries = listing::list(&conn, dir).await?;
        // The sets' rung files, recorded or found by the sets' own word (T678). A view only:
        // nothing is written by reading.
        let manifest = set_files::adopted(&conn, dir, &manifest, &entries, None).await;
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

        // Every file across every medium is probed concurrently, capped at as many
        // channels as the connection sets aside for ordinary work
        // (`ssh::connection::BRIEF_CHANNELS` — the two of `MAX_CONCURRENT_CHANNELS` that
        // stay held for watching viewers are left alone). `buffered`, not
        // `buffer_unordered`: it starts that many probes at once but still hands results
        // back in the order they were queued, not the order they finish — a slow file
        // must not jump the queue, or the list would read differently from one refresh to
        // the next although nothing on the server changed. Reassembled per medium below
        // purely by count, from `matched.media_files`, which already carries the
        // catalogue's order — no result is matched to the wrong file by a race between
        // futures.
        //
        // The list of what to probe is collected into an owned `Vec` first, rather than
        // chaining `.flat_map()` straight into `stream::iter()`: the latter hits a known
        // rustc closure-inference limitation ("implementation of `FnOnce` is not general
        // enough") once it sits inside a `#[tauri::command]` async fn, because the
        // higher-ranked lifetime the macro-generated handler needs cannot be inferred
        // through that particular chain. Plain ownership sidesteps it entirely.
        let file_jobs: Vec<(String, u64, bool)> = matched
            .media_files
            .iter()
            .flat_map(|files| files.files.iter())
            .map(|f| (f.path.clone(), f.size_bytes, f.exists))
            .collect();
        let probed_files: Vec<FileView> =
            stream::iter(file_jobs.into_iter().map(|(path, size_bytes, exists)| {
                let conn = conn.clone();
                async move {
                    let params = probed(state, &conn, profile, &path, size_bytes, exists).await;
                    file_view(profile, &path, size_bytes, params, exists)
                }
            }))
            .buffered(BRIEF_CHANNELS)
            .collect()
            .await;

        let ladder_jobs: Vec<(String, u64, bool)> = matched
            .media_files
            .iter()
            .flat_map(|files| files.ladders.iter())
            .map(|l| (l.path.clone(), l.size_bytes, l.exists))
            .collect();
        let probed_ladders: Vec<LadderSetView> =
            stream::iter(ladder_jobs.into_iter().map(|(path, size_bytes, exists)| {
                let conn = conn.clone();
                async move { ladder_view(profile, &conn, &path, size_bytes, exists).await }
            }))
            .buffered(BRIEF_CHANNELS)
            .collect()
            .await;

        let mut media_views = Vec::with_capacity(manifest.media.len());
        let mut probed_files = probed_files.into_iter();
        let mut probed_ladders = probed_ladders.into_iter();
        for (media, files) in manifest.media.iter().zip(matched.media_files.iter()) {
            let views: Vec<FileView> = probed_files.by_ref().take(files.files.len()).collect();
            let ladders: Vec<LadderSetView> =
                probed_ladders.by_ref().take(files.ladders.len()).collect();
            // No header probe: nobody is handed these one by one, and a set has a rung file
            // per rung — that many more round trips for nothing a person reads.
            let set_file_views: Vec<FileView> = files
                .set_files
                .iter()
                .map(|f| {
                    file_view(
                        profile,
                        &f.path,
                        f.size_bytes,
                        probe_moov::FileParams::default(),
                        f.exists,
                    )
                })
                .collect();

            // A quality ladder counts towards the medium's size: deleting frees it too. So do
            // the set's rung files (T678).
            let total: u64 = files.files.iter().map(|f| f.size_bytes).sum::<u64>()
                + files.ladders.iter().map(|l| l.size_bytes).sum::<u64>()
                + files.set_files.iter().map(|f| f.size_bytes).sum::<u64>();

            media_views.push(MediaView {
                id: media.id.clone(),
                title: media.title.clone(),
                slug: media.slug.clone(),
                files: views,
                ladders,
                set_files: set_file_views,
                total_bytes: total,
                created_at: media.created_at.clone(),
                set_work: None,
            });
        }

        // Same reasoning for whatever the catalogue does not claim (FR-015): concurrent,
        // order-preserving. A directory is skipped without a network round trip — it has
        // no header, and looking inside it for who knows what would only add more.
        let unrecognized_jobs: Vec<(String, u64, bool)> = matched
            .unrecognized
            .iter()
            .map(|entry| (entry.name.clone(), entry.size_bytes, entry.is_dir))
            .collect();
        let unrecognized: Vec<FileView> = stream::iter(unrecognized_jobs.into_iter().map(
            |(name, size_bytes, is_dir)| {
                let conn = conn.clone();
                async move {
                    let params = if is_dir {
                        probe_moov::FileParams::default()
                    } else {
                        probed(state, &conn, profile, &name, size_bytes, true).await
                    };
                    file_view(profile, &name, size_bytes, params, true)
                }
            },
        ))
        .buffered(BRIEF_CHANNELS)
        .collect()
        .await;

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
    /// Suggest how the unrecognized files might belong together (T480).
    ///
    /// **Written in milestone A and connected to nothing until now.** `domain::grouping` was
    /// built for this very screen — the module's own words are "show them and, where it can,
    /// suggest what is what" — and only the showing half was ever wired. The reachability
    /// guard (T479) found it, which is what that guard is for.
    ///
    /// **The names come from the store, not from the caller.** The screen is showing the same
    /// list, and taking it from there would put the question of what counts as unrecognized on
    /// the side of the screen, where it does not belong.
    ///
    /// A suggestion and nothing more: nothing is written and nothing is grouped. A guessed
    /// connection put into the catalogue unasked diverges from what the person meant, and
    /// untangling that later is harder than grouping by hand.
    pub async fn library_suggest_groups(
        state: &super::super::AppState,
        server_id: &str,
    ) -> Result<Suggestion> {
        let view = library_list(state, server_id, false).await?;
        let names: Vec<String> = view.unrecognized.iter().map(|f| f.path.clone()).collect();
        Ok(crate::domain::grouping::suggest(&names))
    }

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

        let conn = gate::open(state.secrets.as_ref(), &profile, Intent::Change)
            .await?
            .conn;
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
    /// interface must warn about that before calling. When somebody is watching right now,
    /// the rename is refused with `FILE_IN_USE` unless `confirmed` (FR-019a) — the same
    /// mechanism `media_delete` already has for the same reason: a `mv` on the server
    /// would drop an active download without warning.
    ///
    /// **One step on the server** (T620): the generation is checked, the entries are moved
    /// and the catalogue is written under the catalogue's lock, together
    /// (`manifest_io::write_moving`). A catalogue changed by somebody else meanwhile is
    /// `MANIFEST_CONFLICT` with nothing moved — read again and retry works; a move that is
    /// refused moves back what was moved and changes nothing.
    pub async fn media_rename(
        state: &AppState,
        server_id: &str,
        media_id: &str,
        title: Option<&str>,
        slug: Option<&str>,
        confirmed: bool,
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

        let conn = gate::open(state.secrets.as_ref(), &profile, Intent::Change)
            .await?
            .conn;
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
        let mut moves: Vec<(String, String)> = Vec::new();
        // The set's rung files move with the short name (T678) — a set built before T678
        // included, found by the set's own word and recorded by this write. Only this medium's
        // are taken from the view: the others are not this write's business.
        if new_slug.is_some_and(|s| s != next.media[index].slug) {
            let entries = listing::list(&conn, &profile.video_dir).await?;
            let seen =
                set_files::adopted(&conn, &profile.video_dir, &manifest, &entries, Some(index))
                    .await;
            next.media[index].set_files = seen.media[index].set_files.clone();
        }
        let media = &mut next.media[index];
        if let Some(t) = new_title {
            media.title = t.to_owned();
        }

        if let Some(s) = new_slug {
            let old = media.slug.clone();
            if s != old {
                if !confirmed {
                    let connections = active_connections(&conn).await;
                    if connections > 0 {
                        conn.close().await;
                        return Err(AppError::new(ErrorCode::FileInUse)
                            .with_cause(format!("connections={connections}")));
                    }
                }
                // ⚠ **T599 — same guard T596 gave `media_delete`/`file_delete`.**
                // `rename_entries` below runs `mv -n` on the very same top-level paths a
                // `ladder_build` may be mid-encode into, or an `upload_start` may be
                // mid-transfer to — a rename racing either moves the directory a running
                // task is actively writing, corrupting it exactly as an unguarded `rm -rf`
                // would have. Checked against `media.all_paths()` (the medium's current
                // `files`/`ladders`, the same source `media_delete` reads `tops_of` from),
                // not `old` alone, for the same reason `refuse_if_busy` is already checked
                // that way elsewhere: a medium's set of top-level names is not guaranteed
                // to collapse to a single slug in every layout.
                //
                // ⚠ **T606 — and against every DESTINATION too.** The new names are worked
                // out here, before the guard, rather than inside the `mv` loop: a rename
                // `film`→`fresh` moving `film_9.mp4` onto `fresh_9.mp4` while an upload of a
                // new `fresh_9.mp4` is still in transfer would have the upload's final
                // `mv -f` overwrite the renamed file the moment it finished; the same for a
                // build writing `fresh/` while `film/` is moved onto it. `slug_available`
                // only knows the catalogue, not what running tasks are about to create.
                //
                // The old and the new short name themselves are asked about as well, even
                // when no path of the medium is (or becomes) exactly `old`/`s`: a build is
                // keyed by slug, and a medium holding only `film_9.mp4` has no top equal to
                // `film` or `fresh`. A build of `fresh` running while `film` becomes `fresh`
                // would have its finished set filed under the renamed medium by
                // `attach_built_set` (`find_by_slug`) — a set nobody built for it; a build of
                // `film` would find no medium left to file its set under.
                let plan = media::rename_plan(media, &old, s);
                let mut touched = tops_of(media.all_paths());
                let slugs = [old.clone(), s.to_owned()];
                for target in tops_of(plan.targets()).into_iter().chain(slugs) {
                    if !touched.contains(&target) {
                        touched.push(target);
                    }
                }
                if let Some(err) = refuse_if_busy(state, server_id, &touched, ErrorCode::MediaBusy)?
                {
                    conn.close().await;
                    return Err(err);
                }
                // ⚠ **T620 — the moves go with the catalogue write, not before it.** They
                // used to run here, one `mv` per entry, and the catalogue was written after:
                // a catalogue somebody changed in between was refused as `MANIFEST_CONFLICT`
                // with the files already under their new names, and "read again and
                // retry" looked for the old ones. Now `write_moving` checks the generation,
                // moves, and writes in one step on the server under the catalogue's lock
                // (the same lock T604 put on every catalogue write); a conflict stops it
                // before anything is moved, and a failed move moves back what was moved.
                moves = plan.renames;
                media.files = plan.files;
                media.ladders = plan.ladders;
                media.set_files = plan.set_files;
                media.slug = s.to_owned();
            }
        }

        manifest_io::write_moving(
            &conn,
            &profile.video_dir,
            &next,
            manifest.generation,
            &moves,
        )
        .await?;
        conn.close().await;

        invalidate(state, server_id);
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
        let conn = gate::open(state.secrets.as_ref(), &profile, Intent::Change)
            .await?
            .conn;
        let manifest = manifest_io::read(&conn, &profile.video_dir).await?;

        let Some(index) = manifest.media.iter().position(|m| m.id == media_id) else {
            conn.close().await;
            return Err(no_such_media(media_id));
        };

        // **The set's rung files go with the medium** (T678): those the catalogue records,
        // and those of a set built before T678, found by the set's own word. The same view
        // the library shows and the confirmation names — what is named is what is removed.
        // A listing that fails stops the deletion: without it the recorded rung files could
        // not be checked, and would be left behind unsaid.
        let entries = match listing::list(&conn, &profile.video_dir).await {
            Ok(entries) => entries,
            Err(e) => {
                conn.close().await;
                return Err(e.into());
            }
        };
        let seen =
            set_files::adopted(&conn, &profile.video_dir, &manifest, &entries, Some(index)).await;

        if !confirmed {
            let impact = impact_of(&conn, &seen, &entries, index).await;
            conn.close().await;
            return Err(confirmation_needed(&seen.media[index].title, &impact));
        }

        // ⚠ **T596 — checked before a single byte moves, the same as the confirmation
        // refusal just above.** `running_build_for`/`running_upload_for` (T591) only ever
        // guarded one direction: a second task could not START on top of a running one. A
        // `ladder_build` for this medium's slug writing `video_dir/slug/v22` right now, or an
        // `upload_start` still writing this medium's file, was never asked before `rm -rf`
        // ran straight through it — found by an independent QA audit 2026-09-11 (round 16),
        // the same severity class as T593's live-server corruption.
        if let Some(err) = refuse_if_busy(
            state,
            server_id,
            &tops_of(seen.media[index].all_paths()),
            ErrorCode::MediaBusy,
        )? {
            conn.close().await;
            return Err(err);
        }

        let media = seen.media[index].clone();
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
    ///
    /// `confirmed` is accepted for the shape of the contract to match `media_rename` and
    /// `media_delete`, but is not acted on: FILE_IN_USE exists to warn before an operation
    /// that can drop an active download (a `mv` or an `rm` on the server). This one issues
    /// neither — the file's bytes and its path are untouched, only a JSON record of which
    /// medium owns it changes — so there is nothing an active viewer could be cut off from.
    pub async fn file_move(
        state: &AppState,
        server_id: &str,
        path: &str,
        to_media_id: &str,
        _confirmed: bool,
    ) -> Result<()> {
        let profile = profile_of(state, server_id)?;
        let conn = gate::open(state.secrets.as_ref(), &profile, Intent::Change)
            .await?
            .conn;
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

        // The same transformation the end of an upload uses (T505). It used to be written
        // out here and again there, in two copies of one rule about a catalogue's soundness.
        let Some(next) = manifest.with_file_under(to_media_id, path, path.ends_with(".m3u8"))
        else {
            conn.close().await;
            return Err(no_such_media(to_media_id));
        };

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
        let conn = gate::open(state.secrets.as_ref(), &profile, Intent::Change)
            .await?
            .conn;

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
                set_files: Vec::new(),
            };
            conn.close().await;
            return Err(confirmation_needed(path, &impact));
        }

        // ⚠ **T596 — same guard as `media_delete`, for the same reason.** `file_delete`
        // never called `running_build_for`/`running_upload_for` at all: a single top-level
        // path is exactly what a `ladder_build` or `upload_start` might be writing right
        // now, and `remove_entries` below runs `rm -rf` straight through it unasked.
        if let Some(err) = refuse_if_busy(state, server_id, &[top.to_owned()], ErrorCode::FileBusy)?
        {
            conn.close().await;
            return Err(err);
        }

        remove_entries(&conn, &profile.video_dir, std::iter::once(&path.to_owned())).await?;

        // It leaves the catalogue in the same act: the file is gone, and a reference to it
        // in the catalogue would turn into a file forever missing.
        let manifest = manifest_io::read(&conn, &profile.video_dir).await?;
        if manifest
            .media
            .iter()
            .any(|m| m.all_paths().any(|p| p == path))
        {
            let mut next = manifest.prepared_for_write();
            for m in &mut next.media {
                m.files.retain(|p| p != path);
                m.ladders.retain(|p| p != path);
                m.set_files.retain(|p| p != path);
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
        // The set's rung files, by name (T678): they are counted above, and a person who
        // never made them by hand is owed what they are.
        if !impact.set_files.is_empty() {
            error = error.with_detail(
                Detail::new(DetailCode::ConfirmDeleteSetFiles)
                    .with("count", impact.set_files.len())
                    .with("names", impact.set_files.join(", ")),
            );
        }
        error.with_cause(format!(
            "files={}, bytes={}, connections={}",
            impact.files, impact.bytes, impact.active_connections
        ))
    }

    async fn impact_of(
        conn: &Connection,
        seen: &Manifest,
        entries: &[listing::Entry],
        index: usize,
    ) -> DeletionImpact {
        let media = &seen.media[index];
        let matched = reconcile::reconcile(seen, entries);

        let files = matched.media_files.get(index);
        let bytes = files.map_or(0, |f| {
            f.files.iter().map(|x| x.size_bytes).sum::<u64>()
                + f.ladders.iter().map(|x| x.size_bytes).sum::<u64>()
                + f.set_files.iter().map(|x| x.size_bytes).sum::<u64>()
        });

        DeletionImpact {
            files: media.files.len() + media.ladders.len() + media.set_files.len(),
            bytes,
            active_connections: active_connections(conn).await,
            set_files: media.set_files.clone(),
        }
    }

    /// How many connections the web server is serving right now (FR-019a).
    ///
    /// It lives in `server::active_use`: the same thing is needed before an upload (FR-037),
    /// and two copies of one count would diverge at the first edit.
    pub(crate) async fn active_connections(conn: &Connection) -> usize {
        crate::server::active_use::serving_connections(conn).await
    }

    /// The top-level entries a set of paths would touch — the same grouping
    /// `remove_entries` itself uses just below, pulled out so the T596 guard can be checked
    /// against exactly the set of names `rm -rf` is about to be given, rather than against
    /// the finer paths a medium happens to record.
    fn tops_of<'a>(paths: impl Iterator<Item = &'a String>) -> Vec<String> {
        let mut tops: Vec<String> = Vec::new();
        for path in paths {
            let top = path.split('/').next().unwrap_or(path).to_owned();
            if !tops.contains(&top) {
                tops.push(top);
            }
        }
        tops
    }

    /// Refuse a deletion whose target a running task is actively writing (T596).
    ///
    /// **What T591/T593 protect and what they never did.** `running_build_for`
    /// (`ladder.rs`) and `running_upload_for` (`upload.rs`) exist to stop a SECOND task
    /// from starting on top of a running one; nothing before this called either of them from
    /// the deletion side, so `media_delete`/`file_delete` ran `rm -rf` straight through a
    /// directory a `ladder_build` was mid-encode into, or a file an `upload_start` was
    /// mid-transfer to. Same severity as T593 (a live server, corrupted rather than merely
    /// inconvenienced).
    ///
    /// **Why both guards run against every top, not one each.** A medium's `slug` is
    /// ordinarily the same string as the top of each of its `ladders` paths, but that is a
    /// convention `ladder.rs` follows, not a rule this function is in a position to trust —
    /// and `running_upload_for` keys on the remote *file* name, which for a `files` entry is
    /// the very same top. Running both checks against every top is two cheap in-memory scans
    /// with no case left unguarded, rather than a guess at which check "belongs" to which
    /// kind of path.
    pub(crate) fn refuse_if_busy(
        state: &AppState,
        server_id: &str,
        tops: &[String],
        code: ErrorCode,
    ) -> Result<Option<AppError>> {
        for top in tops {
            if let Some(busy) =
                crate::commands::ladder::api::running_build_for(state, server_id, top)?
            {
                return Ok(Some(
                    AppError::new(code)
                        .with_detail(
                            Detail::new(DetailCode::MediaBusyBuilding).with("slug", top.clone()),
                        )
                        .with_cause(busy),
                ));
            }
            if let Some(busy) =
                crate::commands::upload::api::running_upload_for(state, server_id, top)?
            {
                return Ok(Some(
                    AppError::new(code)
                        .with_detail(
                            Detail::new(DetailCode::MediaBusyUploading).with("name", top.clone()),
                        )
                        .with_cause(busy),
                ));
            }
        }
        Ok(None)
    }

    /// Delete catalogue entries — both files and quality-ladder directories.
    pub(crate) async fn remove_entries<'a>(
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
    ///
    /// A thin wrapper (T578): the two lines now live on `AppState::invalidate_library`,
    /// shared with `upload_start` and `ladder_build`'s background code, which cannot reach
    /// this function's `state: &AppState` at all.
    fn invalidate(state: &AppState, server_id: &str) {
        state.invalidate_library(server_id);
    }
}
