//! Ядро VRCast Studio.
//!
//! Здесь вся логика и всё общение с внешним миром: сервер, файлы, задачи.
//! Интерфейс не знает ни про SSH, ни про FFmpeg — он общается с ядром только
//! через слой команд (см. `specs/001-vrcast-studio/contracts/ipc-commands.md`).

use tauri::Manager;

pub mod commands;
pub mod domain;
pub mod logging;
pub mod server;
pub mod ssh;
pub mod store;
pub mod tasks;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Первым делом — журнал с вырезанием секретов. До этой строки писать в журнал нельзя:
    // всё, что выведено раньше, пройдёт мимо защиты (конституция, принцип IV).
    logging::init();

    let state = match commands::AppState::bootstrap() {
        Ok(s) => s,
        Err(e) => {
            // Без локального хранилища работать нельзя: задачи не переживут перезапуск,
            // а профили негде держать. Честнее не запуститься, чем притвориться рабочим.
            tracing::error!(error = %e, "не удалось подготовить хранилища");
            eprintln!(
                "{}
{}",
                e.message, e.hint
            );
            std::process::exit(1);
        }
    };

    let engine = state.tasks.clone();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        // Выбор файла — системным окном. Своё не годится: в веб-окне у выбранного
        // файла нет пути на диске, а заливке нужен именно путь.
        .plugin(tauri_plugin_dialog::init())
        // Уведомление о конце длительной задачи, когда окна не видно (FR-084).
        .plugin(tauri_plugin_notification::init())
        .manage(state)
        .setup(move |app| {
            commands::events::bridge_task_events(app.handle().clone(), &engine);
            let state: tauri::State<'_, commands::AppState> = app.state();
            commands::events::bridge_app_events(app.handle().clone(), &state);

            // Заливки прошлого запуска поднимаются здесь, а НЕ в `AppState::bootstrap`:
            // поднятие порождает работу в исполнителе, а подготовка оболочки идёт
            // до его появления. Прямой вызов оттуда роняет приложение при запуске —
            // ровно так уже случалось с потоком событий (T027).
            match commands::upload::api::restore_uploads(&state) {
                Ok(0) => {}
                Ok(n) => tracing::info!(restored = n, "заливки прошлого запуска ждут продолжения"),
                Err(e) => tracing::error!(error = %e, "заливки прошлого запуска не подняты"),
            }

            // Окно создано скрытым и показывается, когда есть что показать: иначе
            // пользователь видит белую вспышку до загрузки интерфейса.
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
            commands::ipc::server_probe_fingerprint,
            commands::servers::ipc::servers_list,
            commands::servers::ipc::server_add,
            commands::servers::ipc::server_update,
            commands::servers::ipc::server_remove,
            commands::servers::ipc::server_set_active,
            commands::servers::ipc::server_test,
            commands::servers::ipc::server_fingerprint_confirm,
            commands::servers::ipc::server_import_suggestion,
            commands::library::ipc::library_list,
            commands::library::ipc::media_create,
            commands::library::ipc::media_rename,
            commands::library::ipc::media_delete,
            commands::library::ipc::file_move,
            commands::library::ipc::file_delete,
            commands::library::ipc::links_for,
            commands::upload::ipc::upload_start,
            commands::upload::ipc::upload_resume,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
