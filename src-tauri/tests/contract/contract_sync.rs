//! Сверка договора между ядром и интерфейсом.
//!
//! Ядро на Rust и его отражение на TypeScript — два описания одного договора. Расходятся
//! они молча: код собирается, типы проверяются, а обработчик ошибки в интерфейсе просто
//! никогда не срабатывает, потому что ждёт код, которого больше нет. Обнаружится это
//! у пользователя, в тот момент, когда ошибка наконец случится.
//!
//! Поэтому расхождение ловится здесь — при сборке. Два правила, выведенные из
//! ревизии 2026-08-25:
//!
//! 1. Каждая сверка — В ОБЕ СТОРОНЫ. Односторонняя ловит «в ядре есть, в TS нет»,
//!    но пропускает лишнее в TS — обработчик события, которого ядро не шлёт.
//! 2. Искать значения только ВНУТРИ разобранного объявления, а не по всему файлу:
//!    `contains` по файлу засчитывал бы код, оставшийся в комментарии или в чужом типе.
//!
//! Перечни со стороны Rust берутся из `ALL`, которые порождены тем же макросом,
//! что и сами enum, — рукописного списка, способного отстать, больше нет.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use vrcast_studio_lib::commands::error::ErrorCode;
use vrcast_studio_lib::tasks::state::{TaskKind, TaskState};

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

/// Все строковые литералы из объявления, начинающегося с `marker` и закрытого `;`.
///
/// Комментарии выбрасываются построчно ДО поиска кавычек и точки с запятой: кавычка
/// в комментарии не должна расширять перечень, а `;` в нём — обрезать разбираемый
/// блок. (В значениях договора не бывает ни `//`, ни `;` — на это разбор и опирается.)
fn declared_strings(ts: &str, marker: &str) -> HashSet<String> {
    let start = ts
        .find(marker)
        .unwrap_or_else(|| panic!("в contract.ts нет объявления «{marker}»"));
    let body = &ts[start + marker.len()..];

    let mut clean = String::new();
    let mut closed = false;
    for line in body.lines() {
        let line = line.split("//").next().unwrap_or("");
        if let Some(i) = line.find(';') {
            clean.push_str(&line[..i]);
            closed = true;
            break;
        }
        clean.push_str(line);
        clean.push('\n');
    }
    assert!(closed, "объявление «{marker}» не закрыто точкой с запятой");

    clean
        .split('"')
        .skip(1)
        .step_by(2)
        .map(str::to_owned)
        .collect()
}

/// Сверка перечня в обе стороны с понятным отчётом о каждом направлении.
fn assert_same_sets(what: &str, rust: HashSet<String>, ts: HashSet<String>) {
    let mut missing: Vec<_> = rust.difference(&ts).collect();
    let mut extra: Vec<_> = ts.difference(&rust).collect();
    missing.sort();
    extra.sort();

    assert!(
        missing.is_empty() && extra.is_empty(),
        "{what}: договор разошёлся.\n\
         Есть в ядре, нет в contract.ts: {missing:?}\n\
         Есть в contract.ts, но ядро не выдаёт: {extra:?}"
    );
}

#[test]
fn коды_ошибок_совпадают_в_обе_стороны() {
    let rust: HashSet<String> = ErrorCode::ALL
        .iter()
        .map(|c| c.as_str().to_owned())
        .collect();
    let ts = declared_strings(&contract_ts(), "export type ErrorCode =");
    assert_same_sets("коды ошибок", rust, ts);
}

#[test]
fn виды_задач_совпадают_в_обе_стороны() {
    let rust: HashSet<String> = TaskKind::ALL
        .iter()
        .map(|k| k.as_str().to_owned())
        .collect();
    let ts = declared_strings(&contract_ts(), "export type TaskKind =");
    assert_same_sets("виды задач", rust, ts);
}

#[test]
fn состояния_задач_совпадают_в_обе_стороны() {
    let rust: HashSet<String> = TaskState::ALL
        .iter()
        .map(|s| s.as_str().to_owned())
        .collect();
    let ts = declared_strings(&contract_ts(), "export type TaskState =");
    assert_same_sets("состояния задач", rust, ts);
}

#[test]
fn имена_событий_совпадают_в_обе_стороны() {
    use vrcast_studio_lib::commands::events::names;

    // Перечень имён здесь рукописный: у модуля names нет своего ALL. Забытое
    // здесь имя поймает обратная сторона — лишнее значение в EVENTS.
    let rust: HashSet<String> = [
        names::TASK_PROGRESS,
        names::TASK_DONE,
        names::LIBRARY_CHANGED,
        names::SERVER_STATE,
        names::VIEWERS_UPDATE,
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();

    let ts = declared_strings(&contract_ts(), "export const EVENTS = {");
    assert_same_sets("имена событий", rust, ts);
}
