//! The core of VRCast Studio.
//!
//! All the logic and all the dealings with the outside world live here: the server,
//! files, tasks. The interface knows nothing of SSH or FFmpeg — it talks to the core
//! only through the command layer (see
//! `specs/001-vrcast-studio/contracts/ipc-commands.md`).

use tauri::Manager;

pub mod commands;
pub mod domain;
pub mod error;
pub mod logging;
pub mod media;
pub mod server;
pub mod ssh;
pub mod store;
pub mod tasks;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // First of all, the log with secret redaction. Nothing may be logged before this
    // line: anything written earlier goes past the guard (constitution, principle IV).
    logging::init();

    let state = match commands::AppState::bootstrap() {
        Ok(s) => s,
        Err(e) => {
            // Without local storage there is no working: tasks would not survive a
            // restart and there would be nowhere to keep profiles. Refusing to start
            // is more honest than pretending to work.
            tracing::error!(error = %e, "could not prepare the stores");
            // No catalogue and no window exist yet, so there is no language to
            // choose between. What goes out is the code and the particulars: they can
            // be searched for, which a translated sentence in the wrong language
            // could not be.
            eprintln!("{e}");
            std::process::exit(1);
        }
    };

    // The key viewers' pseudonyms are made with (T222). Set before anything is watched,
    // and before anything could be written down: an address that reaches the log ahead of
    // this line goes in as itself.
    match crate::store::settings::pseudonym_key(&state.db) {
        Ok(key) => crate::store::redact::use_pseudonym_key(key),
        Err(e) => {
            // Not a reason to refuse to start: without a key one is made up for this run,
            // and the only thing lost is that today's tokens will not match yesterday's.
            tracing::warn!(error = %e, "the pseudonym key could not be read; using one made for this run");
        }
    }

    let engine = state.tasks.clone();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        // File choosing through the system dialog. A web one will not do: a file
        // chosen there has no path on disk, and a path is what an upload needs.
        .plugin(tauri_plugin_dialog::init())
        // A notification when a long task ends and the window is out of sight (FR-084).
        .plugin(tauri_plugin_notification::init())
        .manage(state)
        .setup(move |app| {
            commands::events::bridge_task_events(app.handle().clone(), &engine);
            let state: tauri::State<'_, commands::AppState> = app.state();
            commands::events::bridge_app_events(app.handle().clone(), &state);

            // The tables of places, if the month has turned. In the background and from
            // here rather than from `AppState::bootstrap`, for the same reason as the
            // uploads below: it spawns work on the runtime, and the shell is prepared
            // before the runtime exists.
            commands::geo::api::refresh_in_background(&state);

            // Uploads from the previous run are restored here and NOT in
            // `AppState::bootstrap`: restoring spawns work on the runtime, and the
            // shell is prepared before the runtime exists. Calling it from there
            // crashes the application at start-up — which is exactly what already
            // happened with the event stream (T027).
            match commands::upload::api::restore_uploads(&state) {
                Ok(0) => {}
                Ok(n) => {
                    tracing::info!(restored = n, "uploads from the previous run await resuming")
                }
                Err(e) => {
                    tracing::error!(error = %e, "uploads from the previous run were not restored")
                }
            }

            // The window is created hidden and shown when there is something to show:
            // otherwise a person sees a white flash before the interface loads.
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::ipc::app_versions,
            commands::ipc::tasks_list,
            commands::ipc::task_get,
            commands::ipc::task_cancel,
            commands::ipc::task_pause,
            commands::ipc::task_resume,
            commands::ipc::tasks_reorder,
            commands::ipc::tasks_queue_order,
            commands::ipc::tasks_on_close,
            commands::ipc::ffmpeg_probe_self,
            commands::ipc::source_probe,
            commands::ipc::server_probe_fingerprint,
            commands::servers::ipc::servers_list,
            commands::servers::ipc::server_add,
            commands::servers::ipc::server_update,
            commands::servers::ipc::server_remove,
            commands::servers::ipc::server_set_active,
            commands::servers::ipc::server_test,
            commands::servers::ipc::server_fingerprint_confirm,
            commands::servers::ipc::server_import_suggestion,
            commands::viewers::ipc::viewers_watch_start,
            commands::viewers::ipc::viewers_watch_stop,
            commands::viewers::ipc::viewers_history,
            commands::settings::ipc::settings_get,
            commands::settings::ipc::settings_set,
            commands::geo::ipc::geo_status,
            commands::geo::ipc::geo_update,
            commands::library::ipc::library_list,
            commands::library::ipc::media_create,
            commands::library::ipc::media_rename,
            commands::library::ipc::media_delete,
            commands::library::ipc::file_move,
            commands::library::ipc::file_delete,
            commands::library::ipc::links_for,
            commands::limits::ipc::limit_preview,
            commands::limits::ipc::limit_set,
            commands::limits::ipc::limit_clear,
            commands::limits::ipc::limits_list,
            commands::ladder::ipc::ladder_measure,
            commands::ladder::ipc::ladder_plan,
            commands::ladder::ipc::ladder_validate,
            commands::quality::ipc::quality_measure_preview,
            commands::quality::ipc::quality_measure_start,
            commands::quality::ipc::quality_measure_result,
            commands::quality::ipc::quality_measurements,
            commands::quality::ipc::quality_measure_reuse,
            commands::quality::ipc::quality_measure_forget,
            commands::convert::ipc::convert_preview,
            commands::convert::ipc::convert_start,
            commands::convert::ipc::convert_validate,
            commands::upload::ipc::upload_start,
            commands::upload::ipc::upload_resume,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
