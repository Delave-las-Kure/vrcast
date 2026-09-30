//! The words of the start-up failure message (T655), in both languages — a catalogue and
//! nothing else.
//!
//! **Why here and not in `src/shared/i18n`.** Everything else a person reads comes from those
//! catalogues, through the interface. This message is shown precisely when the interface
//! cannot be: the database has failed to open, no window exists, and there is no WebView to
//! read a catalogue with. So the core keeps these few sentences itself — in this one file,
//! apart from the code, so that they can be reworded or translated without touching it.
//! `tests/unit/code_language.rs` excuses this file by name (`startup_words.rs`), and only it.
//!
//! Placeholders in braces are filled by `fill`: `{file}`, `{folder}`, `{found}`, `{known}`,
//! `{version}`, `{why}`.

pub(super) struct Words {
    pub title: &'static str,
    pub reveal: &'static str,
    pub close: &'static str,
    pub unknown_path: &'static str,

    pub what_not_a_database: &'static str,
    pub what_too_new: &'static str,
    pub what_no_access: &'static str,
    pub what_no_data_dir: &'static str,
    pub what_upgrade_failed: &'static str,
    pub what_other: &'static str,

    pub db_file: &'static str,
    pub log: &'static str,
    pub log_not_written: &'static str,
    pub untouched: &'static str,
    pub what_to_do: &'static str,
    pub details: &'static str,

    pub step_close: &'static str,
    pub step_keep_copy: &'static str,
    pub step_run_newer: &'static str,
    pub step_start_afresh: &'static str,
    pub step_check_access: &'static str,
    pub step_start_again: &'static str,
    pub step_check_data_dir: &'static str,
    pub step_send_log: &'static str,
}

pub(super) const RU: Words = Words {
    title: "VRCast Studio не может запуститься",
    reveal: "Показать файл базы",
    close: "Закрыть",
    unknown_path: "(не определён)",

    what_not_a_database:
        "Файл локальной базы повреждён или не является базой данных VRCast Studio.",
    what_too_new: "Локальную базу записала более новая версия VRCast Studio (схема {found}). \
                   Эта версия ({version}) понимает схему не новее {known} и не открывает базу, \
                   чтобы не повредить её.",
    what_no_access: "Нет доступа к файлу локальной базы: его не удаётся открыть или записать.",
    what_no_data_dir: "Не удалось определить папку данных приложения для этой учётной записи.",
    what_upgrade_failed: "Не удалось обновить локальную базу до этой версии приложения.",
    what_other: "Не удалось открыть локальную базу.",

    db_file: "Файл базы: ",
    log: "Журнал: ",
    log_not_written: "не записывается ({why})",
    untouched: "Приложение ничего не удаляло и не меняло в этом файле.",
    what_to_do: "Что делать:",
    details: "Подробности: ",

    step_close: "Закройте VRCast Studio.",
    step_keep_copy: "Сохраните копию файла {file} (и файлов {file}-wal, {file}-shm, если они есть \
                     рядом) в надёжное место — в нём ваши профили серверов и задачи.",
    step_run_newer: "Установите и запустите версию VRCast Studio не старше той, что работала с \
                     этой базой в последний раз (новее {version}).",
    step_start_afresh: "Чтобы начать с чистой базы, переименуйте {file} (например, в \
                        {file}.broken) и запустите приложение снова: оно создаст новую пустую \
                        базу. Профили серверов придётся добавить заново.",
    step_check_access: "Проверьте, что файл {file} и папка {folder} не только для чтения, \
                        доступны вашей учётной записи и не заняты другой программой (антивирус, \
                        синхронизация облака, ещё одна копия VRCast Studio).",
    step_start_again: "Запустите приложение снова.",
    step_check_data_dir: "Проверьте, что у учётной записи есть домашняя папка и папка данных \
                          приложений (на Windows — %APPDATA%), затем запустите приложение снова.",
    step_send_log: "Если не помогло — пришлите журнал разработчикам.",
};

pub(super) const EN: Words = Words {
    title: "VRCast Studio cannot start",
    reveal: "Show the database file",
    close: "Close",
    unknown_path: "(unknown)",

    what_not_a_database: "The local database file is damaged or is not a VRCast Studio database.",
    what_too_new: "The local database was written by a newer version of VRCast Studio (schema \
                   {found}). This version ({version}) understands schema {known} at most and \
                   will not open it, so as not to damage it.",
    what_no_access: "The local database file cannot be reached: it cannot be opened or written.",
    what_no_data_dir: "The application data folder for this user account could not be \
                       determined.",
    what_upgrade_failed: "The local database could not be brought up to this version of the \
                          application.",
    what_other: "The local database could not be opened.",

    db_file: "Database file: ",
    log: "Log: ",
    log_not_written: "not being written ({why})",
    untouched: "The application has not deleted or changed anything in this file.",
    what_to_do: "What to do:",
    details: "Details: ",

    step_close: "Close VRCast Studio.",
    step_keep_copy: "Save a copy of {file} (and of {file}-wal and {file}-shm, if they are beside \
                     it) somewhere safe — it holds your server profiles and tasks.",
    step_run_newer: "Install and run a version of VRCast Studio no older than the one that last \
                     used this database (newer than {version}).",
    step_start_afresh: "To start with a clean database, rename {file} (for example to \
                        {file}.broken) and start the application again: it will create a new \
                        empty one. Server profiles will have to be added again.",
    step_check_access: "Check that {file} and the folder {folder} are not read-only, are \
                        accessible to your user account and are not held by another program \
                        (antivirus, cloud sync, another copy of VRCast Studio).",
    step_start_again: "Start the application again.",
    step_check_data_dir: "Check that the user account has a home folder and an application data \
                          folder (on Windows, %APPDATA%), then start the application again.",
    step_send_log: "If that does not help, send the log to the developers.",
};
