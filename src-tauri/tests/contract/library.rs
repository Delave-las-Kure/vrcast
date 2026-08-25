//! T036 — договорные тесты команд библиотеки.
//!
//! Здесь проверяется то, что решается **без сервера**: разбор доводов, коды отказов
//! и построение ссылок. Всё, для чего нужна настоящая опись — занятое короткое имя,
//! требование подтверждения с числом файлов и объёмом, расхождение поколений, —
//! проверяется против настоящего OpenSSH в `tests/integration/library_ops.rs`
//! и `manifest_conflict.rs`.
//!
//! Разделение не формальное: договорный тест, подсовывающий команде выдуманную опись,
//! проверял бы согласие кода с этой выдумкой, а не с тем, что лежит на сервере.

use super::support::{state, valid_input};
use vrcast_studio_lib::commands::error::ErrorCode;
use vrcast_studio_lib::commands::library::{api, FileView, LibraryView, MediaView};
use vrcast_studio_lib::commands::servers::api as servers_api;

const SECRET: &str = "пароль-от-сервера-для-теста-9f3a";

fn state_with_server() -> (vrcast_studio_lib::commands::AppState, String) {
    let s = state();
    let mut input = valid_input("Сервер");
    input.domain = String::from("stream.example.com");
    let id = servers_api::server_add(&s, input, SECRET).expect("профиль не создан");
    (s, id)
}

// ---------- ссылки ----------

#[test]
fn ссылки_на_файл_строятся_из_профиля() {
    // FR-016. Домен берётся из профиля пользователя — в приложении его нет и быть
    // не может (FR-004).
    let (s, id) = state_with_server();

    let links = api::links_for(&s, &id, "Backrooms_22.mp4").expect("ссылки не построены");
    assert_eq!(
        links.origin,
        "https://stream.example.com/videos/Backrooms_22.mp4"
    );
    assert_eq!(links.cdn, None, "CDN не задан, а вторая ссылка появилась");
}

#[test]
fn при_заданном_cdn_отдаются_обе_ссылки() {
    let s = state();
    let mut input = valid_input("С посредником");
    input.cdn_base = Some(String::from("https://cdn.example.net"));
    let id = servers_api::server_add(&s, input, SECRET).unwrap();

    let links = api::links_for(&s, &id, "a.mp4").unwrap();
    assert_eq!(links.origin, "https://stream.example.com/videos/a.mp4");
    assert_eq!(
        links.cdn.as_deref(),
        Some("https://cdn.example.net/videos/a.mp4")
    );
}

#[test]
fn ссылки_для_несуществующего_сервера_это_ошибка() {
    let s = state();
    let err = api::links_for(&s, "нет-такого", "a.mp4").expect_err("выданы ссылки в пустоту");
    assert_eq!(err.code, ErrorCode::InvalidInput);
}

// ---------- разбор доводов, не требующий сервера ----------

#[tokio::test]
async fn медиа_с_недопустимым_коротким_именем_не_создаётся() {
    // Проверка идёт до обращения к серверу: незачем ходить в сеть, чтобы отвергнуть
    // то, что отвергается по виду.
    let (s, id) = state_with_server();

    let err = api::media_create(&s, &id, "Название", Some("имя с пробелом"))
        .await
        .expect_err("создано медиа с пробелом в коротком имени");
    assert_eq!(err.code, ErrorCode::InvalidInput);
    assert!(
        !err.hint.trim().is_empty(),
        "отказ без подсказки, что делать"
    );
}

#[tokio::test]
async fn медиа_с_пустым_названием_не_создаётся() {
    let (s, id) = state_with_server();
    let err = api::media_create(&s, &id, "   ", None)
        .await
        .expect_err("создано медиа без названия");
    assert_eq!(err.code, ErrorCode::InvalidInput);
}

#[tokio::test]
async fn название_без_латинского_соответствия_требует_короткого_имени_от_человека() {
    // Приложение не выдумывает короткое имя из мусора: оно попадёт в имя файла
    // и в ссылку, и исправлять это будет поздно.
    let (s, id) = state_with_server();
    let err = api::media_create(&s, &id, "日本語", None)
        .await
        .expect_err("короткое имя выдумано из ниоткуда");
    assert_eq!(err.code, ErrorCode::InvalidInput);
    assert!(
        err.message.contains("орот") || err.hint.contains("орот"),
        "отказ не объясняет, что нужно короткое имя: {} / {}",
        err.message,
        err.hint
    );
}

#[tokio::test]
async fn переименование_без_единого_нового_значения_отвергается() {
    // Вызов, который ничего не меняет, но записывает опись, — это лишнее поколение
    // и лишний повод для расхождения с другим экземпляром приложения.
    let (s, id) = state_with_server();
    let err = api::media_rename(&s, &id, "m1", None, None)
        .await
        .expect_err("принято переименование в никуда");
    assert_eq!(err.code, ErrorCode::InvalidInput);
}

#[tokio::test]
async fn команды_библиотеки_для_несуществующего_сервера_отказывают() {
    let s = state();
    for err in [
        api::library_list(&s, "нет-такого", false).await.err(),
        api::media_create(&s, "нет-такого", "Название", None)
            .await
            .err(),
        api::media_delete(&s, "нет-такого", "m1", true).await.err(),
        api::file_delete(&s, "нет-такого", "a.mp4", true)
            .await
            .err(),
    ] {
        let err = err.expect("команда отработала на несуществующем сервере");
        assert_eq!(err.code, ErrorCode::InvalidInput, "неверный код: {err:?}");
    }
}

// ---------- форма ответа ----------

#[test]
fn полнота_библиотеки_считается_по_всем_видимым_файлам() {
    // Свойство, ради которого группа «не распознано» вообще существует (FR-015):
    // число файлов, видимых пользователю, обязано совпадать с числом файлов
    // в каталоге. Файл, не попавший ни в медиа, ни в эту группу, — потерянный файл.
    let view = LibraryView {
        server_id: String::from("srv"),
        media: vec![MediaView {
            id: String::from("m1"),
            title: String::from("Фильм"),
            slug: String::from("film"),
            files: vec![file_view("film_22.mp4"), file_view("film_10.mp4")],
            ladders: vec![String::from("film/master.m3u8")],
            total_bytes: 2048,
            created_at: String::from("2026-08-01T10:00:00Z"),
        }],
        unrecognized: vec![file_view("непонятное.mp4")],
        disk: None,
        stale: false,
    };

    // Два файла медиа, один набор качеств, одно нераспознанное.
    assert_eq!(view.accounted_entries(), 4);
}

#[test]
fn ответ_библиотеки_переживает_передачу_через_границу() {
    // Договор пересекает границу между ядром и интерфейсом в виде JSON. Тип, который
    // не проходит туда и обратно, — это договор, который где-то потеряет данные.
    let view = LibraryView {
        server_id: String::from("srv"),
        media: Vec::new(),
        unrecognized: vec![file_view("одинокий.mp4")],
        disk: Some(vrcast_studio_lib::commands::library::DiskUsage {
            total_bytes: 100,
            free_bytes: 40,
            used_by_videos_bytes: 55,
        }),
        stale: true,
    };

    let json = serde_json::to_string(&view).expect("ответ не сериализуется");
    let back: LibraryView = serde_json::from_str(&json).expect("ответ не читается обратно");
    assert_eq!(back, view);
    assert!(
        json.contains("\"stale\":true"),
        "признак устаревших данных потерян: {json}"
    );
}

fn file_view(path: &str) -> FileView {
    FileView {
        path: path.to_owned(),
        size_bytes: 1024,
        duration_s: None,
        width: None,
        height: None,
        bitrate_bps: None,
        video_codec: None,
        audio_codec: None,
        faststart_ok: None,
        exists_on_server: true,
        origin_url: format!("https://stream.example.com/videos/{path}"),
        cdn_url: None,
    }
}
