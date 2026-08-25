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
use vrcast_studio_lib::commands::error::{DetailCode, ErrorCode};
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
fn коды_уточнений_совпадают_в_обе_стороны() {
    // Уточнения появились вместе с двумя языками: ядро перестало сочинять фразы и
    // теперь называет случай кодом, а формулировку подбирает интерфейс. Забытый
    // здесь код — это пустое место на экране вместо объяснения, и узналось бы это
    // у пользователя.
    //
    // Полноту самих словарей проверяет компилятор TypeScript: они объявлены как
    // `Record<DetailCode, ...>`, и пропущенный ключ роняет сборку интерфейса.
    // Здесь сверяется звено перед этим — что перечень кодов в TS вообще тот же.
    let rust: HashSet<String> = DetailCode::ALL
        .iter()
        .map(|c| c.as_str().to_owned())
        .collect();
    let ts = declared_strings(&contract_ts(), "export type DetailCode =");
    assert_same_sets("коды уточнений", rust, ts);
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
        names::TASK_NOTIFY,
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

// ---------- сверка ФОРМ, а не только перечней (T075) ----------

/// Имена полей объявленного в TypeScript интерфейса.
///
/// Разбор нарочно простой и опирается на то, как этот файл написан: одно поле
/// на строку, `имя: тип;`. Полноценный разбор TypeScript здесь был бы средством
/// не по задаче — а если файл начнут писать иначе, сверка честно упадёт, а не
/// сделает вид, что всё сошлось.
fn declared_fields(ts: &str, name: &str) -> HashSet<String> {
    let marker = format!("export interface {name} {{");
    let start = ts
        .find(&marker)
        .unwrap_or_else(|| panic!("в contract.ts нет интерфейса «{name}»"));
    let body = &ts[start + marker.len()..];
    let end = body
        .find("\n}")
        .unwrap_or_else(|| panic!("интерфейс «{name}» не закрыт"));

    let mut out = HashSet::new();
    for line in body[..end].lines() {
        // Комментарии выбрасываются до разбора: `/** Что-то: и двоеточие */`
        // иначе дало бы поле с именем «Что-то».
        let line = line.trim();
        if line.starts_with("//") || line.starts_with('*') || line.starts_with("/*") {
            continue;
        }
        let Some((left, _)) = line.split_once(':') else {
            continue;
        };
        let field = left.trim().trim_end_matches('?');
        if !field.is_empty() && field.chars().all(|c| c.is_alphanumeric() || c == '_') {
            out.insert(field.to_owned());
        }
    }
    assert!(!out.is_empty(), "у интерфейса «{name}» не нашлось полей");
    out
}

/// Имена полей, которые ядро на самом деле кладёт в JSON.
///
/// Берутся из настоящей сериализации, а не из объявления структуры: значение
/// имеет только то, что уходит за границу. Переименование через `#[serde(rename)]`
/// или пропуск через `skip_serializing_if` объявление не меняют — а договор меняют.
fn serialized_fields<T: serde::Serialize>(value: &T) -> HashSet<String> {
    let json = serde_json::to_value(value).expect("значение не сериализуется");
    let map = json
        .as_object()
        .expect("ожидался объект: сверять поля у не-объекта нечего");
    map.keys().cloned().collect()
}

/// Сверить форму в обе стороны.
fn same_shape(rust: &HashSet<String>, ts: &HashSet<String>, what: &str) {
    let missing_in_ts: Vec<_> = rust.difference(ts).cloned().collect();
    let missing_in_rust: Vec<_> = ts.difference(rust).cloned().collect();

    assert!(
        missing_in_ts.is_empty(),
        "{what}: ядро шлёт поля, которых нет в contract.ts: {missing_in_ts:?}. \
         Интерфейс их не прочитает, и узнается это у пользователя"
    );
    assert!(
        missing_in_rust.is_empty(),
        "{what}: в contract.ts объявлены поля, которых ядро не шлёт: {missing_in_rust:?}. \
         Интерфейс будет ждать того, чего не будет"
    );
}

#[test]
fn форма_задачи_совпадает_в_обе_стороны() {
    // Перечни значений сверялись и раньше, а имена полей — нет. Переименование
    // поля в serde проходило молча: сборка цела, типы сходятся, а интерфейс
    // читает `undefined` там, где ждал число (задолженность T075).
    let record = vrcast_studio_lib::tasks::store::TaskRecord::new(
        "t1",
        TaskKind::Upload,
        Some(String::from("s1")),
    );
    same_shape(
        &serialized_fields(&record),
        &declared_fields(&contract_ts(), "Task"),
        "Task",
    );
}

#[test]
fn форма_событий_о_задачах_совпадает_в_обе_стороны() {
    use vrcast_studio_lib::tasks::engine::TaskEvent;

    let progress = TaskEvent::Progress {
        id: String::from("t1"),
        state: TaskState::Running,
        progress: 0.5,
        stage: Some(DetailCode::StageConverting),
        speed_bps: Some(1),
        eta_s: Some(2),
    };
    // У события есть ещё поле-метка вида (`event`), объявленное и в TypeScript:
    // по нему одно событие отличают от другого, и оно обязано совпасть тоже.
    same_shape(
        &serialized_fields(&progress),
        &declared_fields(&contract_ts(), "TaskProgressEvent"),
        "TaskProgressEvent",
    );

    let done = TaskEvent::Done {
        id: String::from("t1"),
        state: TaskState::Completed,
        error: None,
    };
    same_shape(
        &serialized_fields(&done),
        &declared_fields(&contract_ts(), "TaskDoneEvent"),
        "TaskDoneEvent",
    );
}

#[test]
fn форма_разобранного_исходника_совпадает_в_обе_стороны() {
    use vrcast_studio_lib::domain::source::{AudioTrack, SourceFile};

    let track = AudioTrack {
        index: 0,
        codec: String::from("aac"),
        channels: 2,
        bitrate_bps: Some(256_000),
        language: Some(String::from("rus")),
        title: None,
        is_default: true,
    };
    same_shape(
        &serialized_fields(&track),
        &declared_fields(&contract_ts(), "AudioTrack"),
        "AudioTrack",
    );

    let source = SourceFile {
        path: String::from("/v/a.mp4"),
        size_bytes: 1,
        duration_s: 1.0,
        width: 1920,
        height: 1080,
        fps: 24,
        bitrate_bps: 1,
        peak_bps: None,
        video_codec: String::from("h264"),
        pix_fmt: String::from("yuv420p"),
        color_transfer: None,
        audio_tracks: vec![track],
    };
    same_shape(
        &serialized_fields(&source),
        &declared_fields(&contract_ts(), "SourceFile"),
        "SourceFile",
    );
}

#[test]
fn форма_проверки_воспроизведения_совпадает_в_обе_стороны() {
    let verdict = vrcast_studio_lib::media::validate::classify("");
    same_shape(
        &serialized_fields(&verdict),
        &declared_fields(&contract_ts(), "Validation"),
        "Validation",
    );
}

#[test]
fn форма_сведений_о_ffmpeg_совпадает_в_обе_стороны() {
    let info = vrcast_studio_lib::media::ffmpeg::FfmpegInfo {
        version: String::from("ffmpeg version n8"),
        path: String::from("/x/ffmpeg"),
        has_x264: true,
        hardware: vec![String::from("h264_nvenc")],
    };
    same_shape(
        &serialized_fields(&info),
        &declared_fields(&contract_ts(), "FfmpegInfo"),
        "FfmpegInfo",
    );
}

#[test]
fn разбор_объявления_не_принимает_комментарий_за_поле() {
    // Разбор простой, и его собственная ошибка была бы незаметна: лишнее «поле»
    // из комментария сделало бы сверку вечно красной, а пропущенное — вечно зелёной.
    let ts = "export interface Проба {\n  /** Что-то: с двоеточием */\n  \
              // и строчный комментарий: тоже\n  настоящее: number;\n  \
              необязательное?: string;\n}\n";
    let fields = declared_fields(ts, "Проба");
    assert_eq!(
        fields.len(),
        2,
        "разобрано лишнее или пропущено нужное: {fields:?}"
    );
    assert!(fields.contains("настоящее"));
    assert!(
        fields.contains("необязательное"),
        "знак вопроса не отброшен"
    );
}
