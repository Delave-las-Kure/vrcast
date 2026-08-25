//! Учёт запущенных внешних программ и уборка уцелевших при запуске.
//!
//! Зачем это нужно, коротко: **гарантии на Windows и Linux не равны.**
//!
//! | Как завершилось приложение | Windows | Linux |
//! |---|---|---|
//! | наш код отработал | убиваем сами | убиваем сами |
//! | паника, аварийный выход | ядро закроет объект задания | сигнал ядра прямому потомку |
//! | сигнал на завершение, нехватка памяти | ядро закроет объект задания | сигнал ядра прямому потомку |
//! | внук, порождённый программой | закрыт объектом задания | **не закрыт** |
//!
//! На Windows объект задания держит всё дерево, и держит его ядро. На Linux сигнал при
//! смерти родителя доходит только до прямого потомка; внуков закрыть нечем.
//!
//! Этот учёт закрывает остаток: идентификаторы запущенных программ пишутся в базу, а при
//! следующем запуске приложение проверяет, не уцелел ли кто, и добивает.
//!
//! **Сверка перед убийством обязательна.** Номера процессов переиспользуются, и за старым
//! номером к моменту следующего запуска может стоять совершенно посторонняя программа —
//! браузер пользователя, например. Поэтому перед завершением проверяется, тот ли это
//! процесс, и при несовпадении запись просто забывается.
//!
//! Сверяется **время запуска**, а не имя. Имя лжёт, и это выяснила непрерывная интеграция
//! под Linux 2026-08-25: `sh -c "sleep 300"` заменяет себя на `sleep`, записанное имя
//! перестаёт совпадать, и уцелевшая программа переживала уборку. Вдобавок система
//! показывает не больше пятнадцати знаков имени, так что программа с длинным именем
//! не совпала бы сама с собой. Имя осталось запасным путём — на случай, когда время
//! узнать не удалось.

use super::process::{kill_pid, process_name};
use crate::store::db::{now_rfc3339, Db, DbError};

/// Записать запущенную программу, чтобы её можно было добить после аварии.
pub fn record(db: &Db, pid: u32, program: &str, task_id: Option<&str>) -> Result<(), DbError> {
    // Опознавательный признак снимаем прямо сейчас, пока процесс заведомо жив
    // и это точно он. Позже за этим номером может оказаться кто угодно.
    let identity = crate::tasks::process::process_identity(pid);
    db.with_conn(|c| {
        c.execute(
            "INSERT OR REPLACE INTO running_processes (pid, program, task_id, started_at, identity)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![pid, program, task_id, now_rfc3339(), identity],
        )?;
        Ok(())
    })
}

/// Забыть запись: программа завершилась штатно.
pub fn forget(db: &Db, pid: u32) -> Result<(), DbError> {
    db.with_conn(|c| {
        c.execute("DELETE FROM running_processes WHERE pid = ?1", [pid])?;
        Ok(())
    })
}

/// Итог уборки.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct SweepReport {
    /// Завершено уцелевших от прошлого запуска.
    pub killed: Vec<u32>,
    /// Записи, за номерами которых оказалась посторонняя программа. Не тронуты.
    pub reused: Vec<u32>,
    /// Записи, чьи программы уже завершились сами.
    pub already_gone: Vec<u32>,
}

impl SweepReport {
    pub fn is_clean(&self) -> bool {
        self.killed.is_empty() && self.reused.is_empty()
    }
}

/// Добить программы, уцелевшие от предыдущего запуска приложения.
///
/// Вызывается один раз при старте, до того как появятся новые задачи. После уборки таблица
/// очищается целиком: всё, что в ней было, относилось к прошлому запуску.
pub fn sweep_on_startup(db: &Db) -> Result<SweepReport, DbError> {
    let records: Vec<(u32, String, Option<String>)> = db.with_conn(|c| {
        let mut stmt = c.prepare("SELECT pid, program, identity FROM running_processes")?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, u32>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    })?;

    let mut report = SweepReport::default();

    for (pid, program, recorded_identity) in records {
        // На Windows сверка и завершение делаются ОДНИМ описателем: между
        // раздельными проверкой и убийством система вправе выдать освободившийся
        // номер другой программе, и убита будет она. Уборка идёт при запуске
        // приложения — то есть ровно тогда, когда номера раздаются пачками
        // (задолженность T074).
        #[cfg(windows)]
        if let Some(identity) = recorded_identity.as_deref() {
            match crate::tasks::process::verify_and_terminate(pid, identity) {
                None => report.already_gone.push(pid),
                Some(true) => {
                    tracing::warn!(
                        pid,
                        program = %program,
                        "добита программа, уцелевшая от предыдущего запуска"
                    );
                    report.killed.push(pid);
                }
                Some(false) => {
                    tracing::debug!(
                        pid,
                        ожидалось = %program,
                        "номер процесса переиспользован, запись забыта без завершения"
                    );
                    report.reused.push(pid);
                }
            }
            continue;
        }

        match process_name(pid) {
            None => report.already_gone.push(pid),
            Some(actual) => {
                if same_process(pid, &program, &actual, recorded_identity.as_deref()) {
                    if kill_pid(pid) {
                        tracing::warn!(
                            pid,
                            program = %program,
                            "добита программа, уцелевшая от предыдущего запуска"
                        );
                        report.killed.push(pid);
                    } else {
                        // Не смогли завершить — но и не молчим об этом.
                        tracing::error!(pid, program = %program, "уцелевшую программу не удалось завершить");
                        report.reused.push(pid);
                    }
                } else {
                    // Номер переиспользован посторонней программой. Не трогаем.
                    tracing::debug!(
                        pid,
                        ожидалось = %program,
                        обнаружено = %actual,
                        "номер процесса переиспользован, запись забыта без завершения"
                    );
                    report.reused.push(pid);
                }
            }
        }
    }

    db.with_conn(|c| {
        c.execute("DELETE FROM running_processes", [])?;
        Ok(())
    })?;

    Ok(report)
}

/// Тот ли это процесс, который мы записали, — или за его номером уже кто-то другой.
///
/// Сверяется в первую очередь **время запуска**: оно не меняется при подмене образа
/// и не обрезается. Имя — запасной путь на случай, когда время узнать не удалось
/// (старая запись без него, не-Linux Unix, отказ в доступе).
///
/// Почему имени недостаточно, выяснила непрерывная интеграция под Linux 2026-08-25:
/// `sh -c "sleep 300"` заменяет себя на `sleep`, записанное имя перестаёт совпадать,
/// и уцелевшая программа переживала уборку. Вдобавок `/proc/<pid>/comm` обрезан
/// пятнадцатью знаками — программа с длинным именем не совпала бы сама с собой.
fn same_process(
    pid: u32,
    recorded_name: &str,
    actual_name: &str,
    recorded_identity: Option<&str>,
) -> bool {
    if let Some(recorded) = recorded_identity {
        if let Some(actual) = crate::tasks::process::process_identity(pid) {
            return actual == recorded;
        }
    }
    names_match(actual_name, recorded_name)
}

/// Совпадают ли имена программ.
///
/// Сравниваем без учёта расширения и регистра: в записи может стоять `ffmpeg`, а система
/// покажет `ffmpeg.exe`. Учитываем и обрезание: система показывает не больше
/// пятнадцати знаков имени, и длинное имя иначе не совпало бы само с собой.
fn names_match(actual: &str, recorded: &str) -> bool {
    /// Сколько знаков имени показывает система. Больше не покажет никогда.
    const SHOWN_CHARS: usize = 15;

    let norm = |s: &str| {
        let base = std::path::Path::new(s)
            .file_stem()
            .map(|x| x.to_string_lossy().into_owned())
            .unwrap_or_else(|| s.to_owned());
        base.to_lowercase()
    };

    let (a, r) = (norm(actual), norm(recorded));
    if a == r {
        return true;
    }
    // Показанное имя упёрлось в предел — сравниваем столько, сколько видно.
    a.chars().count() == SHOWN_CHARS && r.chars().take(SHOWN_CHARS).collect::<String>() == a
}

#[cfg(test)]
mod tests {
    use super::names_match;

    #[test]
    fn имена_сверяются_без_расширения_и_регистра() {
        assert!(names_match("ffmpeg.exe", "ffmpeg"));
        assert!(names_match("FFmpeg.EXE", "ffmpeg"));
        assert!(names_match("ffmpeg", "C:/tools/ffmpeg.exe"));
        assert!(!names_match("chrome.exe", "ffmpeg"));
        assert!(!names_match("ffmpeg-probe", "ffmpeg"));
    }
}
