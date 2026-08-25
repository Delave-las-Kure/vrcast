//! Договорные тесты команд подготовки (часть T112).
//!
//! Договор: `contracts/ipc-commands.md`, раздел «Подготовка файлов».
//!
//! Здесь пока две команды из четырёх: `convert_start` и `convert_validate` ещё
//! не написаны, и проверять у них нечего. Оставшиеся две закрываются целиком —
//! форма ответа и коды отказов.
//!
//! Проверкам нужен вложенный FFmpeg. Он весит сто сорок мегабайт, в репозиторий
//! не попадает и кладётся командой `npm run ffmpeg`; без него проверка объявляет,
//! что пропущена, вслух — иначе она молча ничего не значила бы.

use vrcast_studio_lib::commands::api;
use vrcast_studio_lib::commands::error::ErrorCode;
use vrcast_studio_lib::media::ffmpeg;

/// Есть ли вложенная сборка. Без неё половине проверок нечего делать.
fn есть_ffmpeg() -> bool {
    if ffmpeg::locate("ffprobe").is_ok() {
        return true;
    }
    eprintln!(
        "ПРОПУЩЕНО: вложенного FFmpeg нет. Выполните `npm run ffmpeg`, \
         чтобы эта проверка что-то проверяла."
    );
    false
}

fn временный_файл(name: &str, содержимое: &[u8]) -> std::path::PathBuf {
    let dir =
        std::env::temp_dir().join(format!("vrcast-convert-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&dir).expect("не создать временный каталог");
    let path = dir.join(name);
    std::fs::write(&path, содержимое).expect("не записать файл");
    path
}

#[tokio::test]
async fn проверка_вложенной_сборки_отдаёт_то_что_обещано_договором() {
    if !есть_ffmpeg() {
        return;
    }
    let info = api::ffmpeg_probe_self()
        .await
        .expect("вложенная сборка не прошла проверку");

    assert!(
        info.version.starts_with("ffmpeg version"),
        "{}",
        info.version
    );
    assert!(!info.path.is_empty(), "не сказано, где лежит сборка");
    assert!(
        info.has_x264,
        "договор обещает отказ без libx264, а сборка объявила его отсутствие успехом"
    );
}

#[tokio::test]
async fn разбор_несуществующего_файла_это_ошибка_ввода() {
    if !есть_ffmpeg() {
        return;
    }
    // Опечатка в пути — не сбой приложения, и интерфейс обязан подсветить поле,
    // а не показать уведомление об ошибке. Различить можно только по коду.
    let err = api::source_probe("F:/такого/файла/нет.mp4")
        .await
        .expect_err("разбор несуществующего файла прошёл");

    assert_eq!(err.code, ErrorCode::InvalidInput);
    assert!(
        !err.message.is_empty(),
        "отказ без человеческой формулировки"
    );
    assert!(!err.hint.is_empty(), "отказ без подсказки, что делать");
}

#[tokio::test]
async fn разбор_не_видео_называет_причину_а_не_ругается_кодами() {
    if !есть_ffmpeg() {
        return;
    }
    // Человек выбрал не тот файл — обычное дело. Сказать надо про файл,
    // а не про то, что разборщик вернул ненулевой код.
    let path = временный_файл("заметки.txt", b"vrcast: not a video at all");

    let err = api::source_probe(&path.to_string_lossy())
        .await
        .expect_err("текстовый файл разобрался как видео");

    assert_eq!(err.code, ErrorCode::InvalidInput);
    assert!(
        err.message.contains("видео") || err.message.contains("разобрать"),
        "по сообщению не понять, в чём дело: {}",
        err.message
    );
    // Жалоба разборщика сохраняется: она непонятна, но её можно найти поиском,
    // а «файл плохой» — нельзя.
    assert!(err.cause.is_some(), "потеряно уточнение от разборщика");

    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[tokio::test]
async fn разобранный_файл_переживает_передачу_через_границу() {
    if !есть_ffmpeg() {
        return;
    }
    // Ответ уходит в интерфейс как есть — значит обязан переноситься без потерь.
    // Собирается из образца, а не из живого файла: живой потребовал бы кодирования
    // на каждом прогоне договорных проверок.
    let образец = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ffprobe-sample.json"),
    )
    .expect("образец ответа не прочитать");

    let src = vrcast_studio_lib::media::probe::parse(&образец, "проба.mp4")
        .expect("образец не разобрался");

    let json = serde_json::to_string(&src).expect("не записалось");
    let back: vrcast_studio_lib::domain::source::SourceFile =
        serde_json::from_str(&json).expect("не прочиталось");
    assert_eq!(back, src);
}
