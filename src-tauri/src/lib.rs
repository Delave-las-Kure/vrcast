//! Ядро VRCast Studio.
//!
//! Здесь вся логика и всё общение с внешним миром: сервер, файлы, задачи.
//! Интерфейс не знает ни про SSH, ни про FFmpeg — он общается с ядром только
//! через слой команд (см. `specs/001-vrcast-studio/contracts/ipc-commands.md`).

pub mod logging;
pub mod ssh;
pub mod store;

// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Первым делом — журнал с вырезанием секретов. До этой строки писать в журнал нельзя:
    // всё, что выведено раньше, пройдёт мимо защиты (конституция, принцип IV).
    logging::init();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![greet])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
