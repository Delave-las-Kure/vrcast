//! Ядро VRCast Studio.
//!
//! Здесь вся логика и всё общение с внешним миром: сервер, файлы, задачи.
//! Интерфейс не знает ни про SSH, ни про FFmpeg — он общается с ядром только
//! через слой команд (см. `specs/001-vrcast-studio/contracts/ipc-commands.md`).

use tauri::Manager;

pub mod commands;
pub mod logging;
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
        .manage(state)
        .setup(move |app| {
            commands::events::bridge_task_events(app.handle().clone(), &engine);

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
            commands::ipc::tasks_on_close,
            commands::ipc::server_probe_fingerprint,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
