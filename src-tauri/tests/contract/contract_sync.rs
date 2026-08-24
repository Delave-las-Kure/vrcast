//! Сверка договора между ядром и интерфейсом.
//!
//! Ядро на Rust и его отражение на TypeScript — два описания одного договора. Расходятся
//! они молча: код собирается, типы проверяются, а обработчик ошибки в интерфейсе просто
//! никогда не срабатывает, потому что ждёт код, которого больше нет. Обнаружится это
//! у пользователя, в тот момент, когда ошибка наконец случится.
//!
//! Поэтому расхождение ловится здесь — при сборке.

use std::path::{Path, PathBuf};
use vrcast_studio_lib::commands::error::ErrorCode;

fn frontend_file(rel: &str) -> PathBuf {
    // Ядро лежит в src-tauri/, интерфейс — рядом, в src/.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("нет родительского каталога у src-tauri")
        .join(rel)
}

fn contract_ts() -> String {
    let path = frontend_file("src/shared/contract.ts");
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("не прочитать {}: {e}", path.display()))
}

#[test]
fn каждый_код_ошибки_ядра_есть_в_описании_для_интерфейса() {
    let ts = contract_ts();
    let mut missing = Vec::new();

    for code in ErrorCode::ALL {
        let quoted = format!("\"{}\"", code.as_str());
        if !ts.contains(&quoted) {
            missing.push(code.as_str());
        }
    }

    assert!(
        missing.is_empty(),
        "коды есть в ядре, но отсутствуют в src/shared/contract.ts: {missing:?}\n\
         Добавьте их в тип ErrorCode — иначе интерфейс не сможет их обработать."
    );
}

#[test]
fn в_описании_для_интерфейса_нет_кодов_которых_ядро_не_выдаёт() {
    // Обратная сверка. Лишний код — обработчик, который никогда не сработает:
    // тихий мёртвый код, который выглядит как забота о пользователе.
    let ts = contract_ts();

    let start = ts
        .find("export type ErrorCode =")
        .expect("в contract.ts нет типа ErrorCode");
    let end = ts[start..]
        .find(';')
        .map(|i| start + i)
        .expect("объявление ErrorCode не закрыто");
    let block = &ts[start..end];

    let known: std::collections::HashSet<&str> =
        ErrorCode::ALL.iter().map(|c| c.as_str()).collect();

    let mut extra = Vec::new();
    for raw in block.split('"').skip(1).step_by(2) {
        if !known.contains(raw) {
            extra.push(raw.to_owned());
        }
    }

    assert!(
        extra.is_empty(),
        "коды есть в src/shared/contract.ts, но ядро их не выдаёт: {extra:?}"
    );
}

#[test]
fn имена_событий_совпадают_с_описанием_для_интерфейса() {
    use vrcast_studio_lib::commands::events::names;
    let ts = contract_ts();

    for name in [
        names::TASK_PROGRESS,
        names::TASK_DONE,
        names::LIBRARY_CHANGED,
        names::SERVER_STATE,
        names::VIEWERS_UPDATE,
    ] {
        assert!(
            ts.contains(&format!("\"{name}\"")),
            "имя события {name} отсутствует в src/shared/contract.ts"
        );
    }
}

#[test]
fn состояния_и_виды_задач_совпадают_с_описанием_для_интерфейса() {
    use vrcast_studio_lib::tasks::state::{TaskKind, TaskState};
    let ts = contract_ts();

    for k in [
        TaskKind::Probe,
        TaskKind::Convert,
        TaskKind::Upload,
        TaskKind::BuildLadder,
        TaskKind::Deploy,
        TaskKind::UpgradeServer,
        TaskKind::Diagnose,
    ] {
        assert!(
            ts.contains(&format!("\"{}\"", k.as_str())),
            "вид задачи {} отсутствует в описании для интерфейса",
            k.as_str()
        );
    }

    for s in [
        TaskState::Queued,
        TaskState::Running,
        TaskState::Paused,
        TaskState::Completed,
        TaskState::Failed,
        TaskState::Cancelled,
    ] {
        assert!(
            ts.contains(&format!("\"{}\"", s.as_str())),
            "состояние задачи {} отсутствует в описании для интерфейса",
            s.as_str()
        );
    }
}
