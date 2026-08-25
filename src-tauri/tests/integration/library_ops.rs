//! Операции над библиотекой против настоящего сервера (T047, T048).
//!
//! Здесь то, что нельзя проверить без описи: занятое короткое имя, требование
//! подтверждения с числом файлов и объёмом, переименование файлов вслед за коротким
//! именем, перенос файла между медиа и удаление.
//!
//! Договорный тест на выдуманной описи проверял бы согласие кода с выдумкой.
//! Здесь опись настоящая — та, которую приложение само записало на сервер.

use super::fixture::{key_path, TestServer, KEY_PASSPHRASE};
use std::sync::Arc;
use vrcast_studio_lib::commands::error::ErrorCode;
use vrcast_studio_lib::commands::library::api as library;
use vrcast_studio_lib::commands::servers::{api as servers, ServerInput};
use vrcast_studio_lib::commands::AppState;
use vrcast_studio_lib::domain::server_profile::AuthKind;
use vrcast_studio_lib::store::db::Db;
use vrcast_studio_lib::store::secrets::InMemorySecretStore;

const VIDEO_DIR: &str = "/var/lib/vrcast/videos";

fn app_state() -> AppState {
    AppState::with_db(
        Arc::new(Db::open_in_memory().unwrap()),
        Arc::new(InMemorySecretStore::new()),
    )
    .expect("состояние приложения не собралось")
}

/// Поднять контейнер, завести профиль и разложить файлы.
async fn setup(files: &[&str]) -> (TestServer, AppState, String) {
    let server = TestServer::start().expect("контейнер не поднялся");
    for name in files {
        server
            .exec_inside(&format!("head -c 2048 /dev/urandom > '{VIDEO_DIR}/{name}'"))
            .unwrap_or_else(|e| panic!("не создать {name}: {e}"));
    }

    let state = app_state();
    let input = ServerInput {
        name: String::from("Контейнер"),
        host: server.host().to_owned(),
        port: server.port,
        user: String::from("root"),
        auth_kind: AuthKind::Key,
        key_path: Some(key_path().to_string_lossy().into_owned()),
        domain: String::from("stream.example.com"),
        video_dir: Some(String::from(VIDEO_DIR)),
        cdn_base: None,
        ipv6_mode: None,
    };
    let id = servers::server_add(&state, input, KEY_PASSPHRASE).expect("профиль не создан");
    confirm_fingerprint(&state, &id, &server).await;
    (server, state, id)
}

/// Пройти тот же путь, что и человек в мастере настройки: узнать отпечаток и
/// подтвердить его.
///
/// Без этого шага приложение не подключается вовсе — учётные данные не отправляются
/// серверу, отпечаток которого не подтверждён (FR-092). Пропустить его в оснастке
/// значило бы проверять поведение, до которого пользователь не доберётся.
pub async fn confirm_fingerprint(state: &AppState, server_id: &str, server: &TestServer) {
    let fingerprint =
        vrcast_studio_lib::commands::api::server_probe_fingerprint(server.host(), server.port)
            .await
            .expect("отпечаток не получен");
    servers::server_fingerprint_confirm(state, server_id, &fingerprint)
        .expect("отпечаток не подтверждён");
}

#[tokio::test]
async fn медиа_создаётся_и_видно_в_библиотеке() {
    let (_server, state, id) = setup(&[]).await;

    let media_id = library::media_create(&state, &id, "Название фильма", None)
        .await
        .expect("медиа не создано");

    let view = library::library_list(&state, &id, true).await.unwrap();
    let media = view
        .media
        .iter()
        .find(|m| m.id == media_id)
        .expect("созданное медиа не видно в библиотеке");

    assert_eq!(media.title, "Название фильма");
    assert_eq!(
        media.slug, "nazvanie-filma",
        "короткое имя составлено не по правилам"
    );
}

#[tokio::test]
async fn занятое_короткое_имя_отвергается_своим_кодом() {
    // Отдельный код нужен, чтобы интерфейс предложил другое имя, а не показал
    // общее сообщение о неполадке.
    let (_server, state, id) = setup(&[]).await;
    library::media_create(&state, &id, "Первое", Some("film"))
        .await
        .unwrap();

    let err = library::media_create(&state, &id, "Второе", Some("film"))
        .await
        .expect_err("заведено второе медиа с тем же коротким именем");
    assert_eq!(err.code, ErrorCode::SlugTaken);
}

#[tokio::test]
async fn удаление_без_подтверждения_называет_последствия() {
    // FR-014. Подтверждать вслепую нечего: пользователь обязан увидеть, сколько
    // файлов исчезнет и сколько места освободится.
    let (server, state, id) = setup(&["film_10.mp4", "film_22.mp4"]).await;

    let media_id = library::media_create(&state, &id, "Фильм", Some("film"))
        .await
        .unwrap();
    // Относим оба файла к медиа.
    for name in ["film_10.mp4", "film_22.mp4"] {
        library::file_move(&state, &id, name, &media_id, true)
            .await
            .expect("файл не отнесён к медиа");
    }

    let err = library::media_delete(&state, &id, &media_id, false)
        .await
        .expect_err("медиа удалено без подтверждения");

    assert_eq!(err.code, ErrorCode::ConfirmationRequired);
    assert!(
        err.message.contains('2') && err.message.contains("файла"),
        "в отказе не названо число файлов: {}",
        err.message
    );
    assert!(
        err.message.contains("КБ") || err.message.contains("Б"),
        "в отказе не назван объём: {}",
        err.message
    );

    // Главное: без подтверждения ничего не произошло.
    let still_there = server
        .exec_inside(&format!("ls {VIDEO_DIR}/film_10.mp4"))
        .is_ok();
    assert!(still_there, "файл удалён, хотя подтверждения не было");
}

#[tokio::test]
async fn подтверждённое_удаление_убирает_и_файлы_и_запись_в_описи() {
    let (server, state, id) = setup(&["film_10.mp4", "film_22.mp4", "чужой.mp4"]).await;

    let media_id = library::media_create(&state, &id, "Фильм", Some("film"))
        .await
        .unwrap();
    for name in ["film_10.mp4", "film_22.mp4"] {
        library::file_move(&state, &id, name, &media_id, true)
            .await
            .unwrap();
    }

    library::media_delete(&state, &id, &media_id, true)
        .await
        .expect("медиа не удалилось");

    for name in ["film_10.mp4", "film_22.mp4"] {
        assert!(
            server
                .exec_inside(&format!("test -e {VIDEO_DIR}/{name}"))
                .is_err(),
            "файл {name} остался на сервере"
        );
    }
    // Чужой файл не тронут: удаление медиа не имеет права задевать соседей.
    assert!(
        server
            .exec_inside(&format!("test -e '{VIDEO_DIR}/чужой.mp4'"))
            .is_ok(),
        "удаление медиа задело посторонний файл"
    );

    let view = library::library_list(&state, &id, true).await.unwrap();
    assert!(
        !view.media.iter().any(|m| m.id == media_id),
        "запись о медиа осталась в описи"
    );
    assert_eq!(
        view.unrecognized.len(),
        1,
        "уцелевший файл потерялся: {view:?}"
    );
}

#[tokio::test]
async fn смена_короткого_имени_переименовывает_файлы() {
    // И ломает прежние ссылки — об этом интерфейс обязан предупредить до вызова.
    // Проверяем, что переименование действительно доходит до сервера: опись,
    // ссылающаяся на несуществующие файлы, хуже отсутствия переименования.
    let (server, state, id) = setup(&["film_10.mp4", "film_22.mp4"]).await;

    let media_id = library::media_create(&state, &id, "Фильм", Some("film"))
        .await
        .unwrap();
    for name in ["film_10.mp4", "film_22.mp4"] {
        library::file_move(&state, &id, name, &media_id, true)
            .await
            .unwrap();
    }

    library::media_rename(&state, &id, &media_id, None, Some("kino"))
        .await
        .expect("переименование не удалось");

    assert!(
        server
            .exec_inside(&format!("test -e {VIDEO_DIR}/kino_10.mp4"))
            .is_ok(),
        "файл не переименован на сервере"
    );
    assert!(
        server
            .exec_inside(&format!("test -e {VIDEO_DIR}/film_10.mp4"))
            .is_err(),
        "прежний файл остался — на диске появилась копия"
    );

    let view = library::library_list(&state, &id, true).await.unwrap();
    let media = view.media.iter().find(|m| m.id == media_id).unwrap();
    assert_eq!(media.slug, "kino");
    let paths: Vec<&str> = media.files.iter().map(|f| f.path.as_str()).collect();
    assert!(
        paths.contains(&"kino_10.mp4") && paths.contains(&"kino_22.mp4"),
        "опись не поспела за переименованием: {paths:?}"
    );
    assert!(
        media.files.iter().all(|f| f.exists_on_server),
        "опись ссылается на несуществующие файлы: {media:?}"
    );
    // Ссылки пересобрались под новое имя — иначе пользователь скопировал бы
    // адрес, которого уже нет.
    assert!(
        media.files.iter().all(|f| f.origin_url.contains("kino_")),
        "ссылки остались на прежнее имя: {media:?}"
    );
}

#[tokio::test]
async fn переименование_только_названия_файлов_не_трогает() {
    // Смена названия — безобидное действие, и ломать из-за него работающие ссылки
    // было бы неожиданностью для пользователя.
    let (server, state, id) = setup(&["film_10.mp4"]).await;
    let media_id = library::media_create(&state, &id, "Фильм", Some("film"))
        .await
        .unwrap();
    library::file_move(&state, &id, "film_10.mp4", &media_id, true)
        .await
        .unwrap();

    library::media_rename(&state, &id, &media_id, Some("Совсем другое название"), None)
        .await
        .expect("переименование не удалось");

    assert!(
        server
            .exec_inside(&format!("test -e {VIDEO_DIR}/film_10.mp4"))
            .is_ok(),
        "файл переименован из-за смены одного лишь названия"
    );
    let view = library::library_list(&state, &id, true).await.unwrap();
    let media = view.media.iter().find(|m| m.id == media_id).unwrap();
    assert_eq!(media.title, "Совсем другое название");
    assert_eq!(media.slug, "film");
}

#[tokio::test]
async fn удаление_файла_без_подтверждения_ничего_не_делает() {
    let (server, state, id) = setup(&["одинокий.mp4"]).await;

    let err = library::file_delete(&state, &id, "одинокий.mp4", false)
        .await
        .expect_err("файл удалён без подтверждения");
    assert_eq!(err.code, ErrorCode::ConfirmationRequired);
    assert!(
        server
            .exec_inside(&format!("test -e '{VIDEO_DIR}/одинокий.mp4'"))
            .is_ok(),
        "файл исчез без подтверждения"
    );

    library::file_delete(&state, &id, "одинокий.mp4", true)
        .await
        .expect("файл не удалился с подтверждением");
    assert!(
        server
            .exec_inside(&format!("test -e '{VIDEO_DIR}/одинокий.mp4'"))
            .is_err(),
        "файл остался после подтверждённого удаления"
    );
}

#[tokio::test]
async fn удалить_опись_через_команду_нельзя() {
    // Опись — служебная запись во владении приложения. Показывать её пользователю
    // мы не показываем, но и защититься от прямого вызова обязаны.
    let (_server, state, id) = setup(&[]).await;
    library::media_create(&state, &id, "Фильм", Some("film"))
        .await
        .unwrap();

    let err = library::file_delete(&state, &id, "library.json", true)
        .await
        .expect_err("опись библиотеки удалена по просьбе интерфейса");
    assert_eq!(err.code, ErrorCode::InvalidInput);
}

#[tokio::test]
async fn удаление_несуществующего_файла_говорит_об_этом_своим_кодом() {
    let (_server, state, id) = setup(&[]).await;
    let err = library::file_delete(&state, &id, "нет-такого.mp4", true)
        .await
        .expect_err("удалён несуществующий файл");
    assert_eq!(err.code, ErrorCode::FileMissingOnServer);
}

#[tokio::test]
async fn второй_экземпляр_приложения_получает_свой_код_отказа() {
    // Тот же конфликт описи, но увиденный сквозь слой команд: интерфейс обязан
    // получить MANIFEST_CONFLICT, чтобы предложить перечитать и повторить,
    // а не общее «внутренняя ошибка».
    let (server, state, id) = setup(&[]).await;
    library::media_create(&state, &id, "Первое", Some("pervoe"))
        .await
        .unwrap();

    // Второй экземпляр меняет опись мимо нас — и поколение уходит вперёд.
    server
        .exec_inside(&format!(
            "sed -i 's/\"generation\": 1/\"generation\": 99/' {VIDEO_DIR}/library.json"
        ))
        .expect("не подменить поколение");

    // Наша команда читает опись заново, поэтому конфликта на ней не будет —
    // конфликт ловится, когда поколение уходит МЕЖДУ чтением и записью.
    // Проверяем это напрямую слоем записи.
    use vrcast_studio_lib::server::manifest_io;
    let conn = vrcast_studio_lib::server::connect(
        state.secrets.as_ref(),
        &vrcast_studio_lib::store::profiles::get(&state.db, &id)
            .unwrap()
            .unwrap(),
    )
    .await
    .expect("не подключиться");

    let прочитано = manifest_io::read(&conn, VIDEO_DIR).await.unwrap();
    server
        .exec_inside(&format!(
            "sed -i 's/\"generation\": 99/\"generation\": 100/' {VIDEO_DIR}/library.json"
        ))
        .expect("не подменить поколение второй раз");

    let err = manifest_io::write(
        &conn,
        VIDEO_DIR,
        &прочитано.prepared_for_write(),
        прочитано.generation,
    )
    .await
    .expect_err("запись прошла поверх чужого изменения");

    let app_err = vrcast_studio_lib::commands::error::AppError::from(err);
    assert_eq!(app_err.code, ErrorCode::ManifestConflict);
    assert!(
        app_err.hint.contains("бнов") || app_err.hint.contains("овтор"),
        "подсказка не предлагает перечитать и повторить: {}",
        app_err.hint
    );
}
